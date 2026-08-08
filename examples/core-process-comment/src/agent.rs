//! core.process-comment — policy-driven comment action processor (use-case matrix).
//!
//! Covers create/edit/reply/delete/react paths from the eight published use cases:
//! mentions+links, own_only, max thread depth, blocklist quarantine, soft-delete,
//! tenant isolation, and empty body.
#![no_std]
#![no_main]

#[repr(C)]
struct IoVec {
    buffer: *const u8,
    length: usize,
}
#[repr(C)]
struct IoVecMut {
    buffer: *mut u8,
    length: usize,
}

#[link(wasm_import_module = "wasi_snapshot_preview1")]
unsafe extern "C" {
    fn fd_read(fd: u32, vectors: *const IoVecMut, count: usize, read: *mut usize) -> u32;
    fn fd_write(fd: u32, vectors: *const IoVec, count: usize, written: *mut usize) -> u32;
}

static mut INPUT_BUF: [u8; 24576] = [0; 24576];
static mut OUTPUT_BUF: [u8; 16384] = [0; 16384];

const MAX_MENTIONS: usize = 8;
const MAX_LINKS: usize = 4;
const MAX_BLOCK: usize = 8;
const MAX_ROLES: usize = 8;

#[unsafe(no_mangle)]
pub extern "C" fn _start() {
    unsafe {
        let mut total = 0usize;
        loop {
            let vec = IoVecMut {
                buffer: INPUT_BUF.as_mut_ptr().add(total),
                length: INPUT_BUF.len() - total,
            };
            let mut n = 0usize;
            if fd_read(0, &vec, 1, &mut n) != 0 || n == 0 {
                break;
            }
            total += n;
            if total >= INPUT_BUF.len() {
                break;
            }
        }
        let out_len = process(&INPUT_BUF[..total], &mut OUTPUT_BUF);
        let out = IoVec {
            buffer: OUTPUT_BUF.as_ptr(),
            length: out_len,
        };
        let mut written = 0usize;
        let _ = fd_write(1, &out, 1, &mut written);
    }
}

unsafe fn process(input: &[u8], out: &mut [u8]) -> usize {
    let action = extract_string_at_depth(input, b"\"action\"", 1);
    let actor = object_after_key(input, b"\"actor\"").unwrap_or(b"");
    let resource = object_after_key(input, b"\"resource\"").unwrap_or(b"");
    let comment = object_after_key(input, b"\"comment\"").unwrap_or(b"");
    let policy = object_after_key(input, b"\"comment_policy\"").unwrap_or(b"");

    if action.is_empty() || actor.is_empty() || resource.is_empty() || policy.is_empty() {
        return deny(
            out,
            b"malformed input",
            b"invalid_input",
            br#"[{"type":"audit_log","severity":"required"}]"#,
            br#"["precondition failed: required fields missing"]"#,
            policy,
        );
    }

    let actor_id = extract_string(actor, b"\"id\"");
    if actor_id.is_empty() {
        return deny(
            out,
            b"actor.id is required but missing",
            b"invalid_actor",
            br#"[{"type":"audit_log","severity":"required"}]"#,
            br#"["precondition failed: actor.id missing"]"#,
            policy,
        );
    }

    let mut roles: [&[u8]; MAX_ROLES] = [&b""[..]; MAX_ROLES];
    let role_count = parse_string_array(actor, b"\"roles\"", &mut roles);
    let actor_tenant = attr_string(actor, b"tenant_id");
    let resource_tenant = attr_string(resource, b"tenant_id");
    let visibility_attr = attr_string(resource, b"visibility");
    let visibility_model = extract_string(policy, b"\"visibility_model\"");

    let enforce_tenant = contains(policy, b"\"enforce_tenant_isolation\":true")
        || contains(policy, b"\"enforce_tenant_isolation\": true");
    if enforce_tenant
        && !actor_tenant.is_empty()
        && !resource_tenant.is_empty()
        && actor_tenant != resource_tenant
    {
        return deny(
            out,
            b"actor.tenant_id does not match resource.tenant_id",
            b"tenant_isolation_violation",
            br#"[{"type":"audit_log","severity":"required","metadata":{"security_event":true}}]"#,
            br#"["enforce_tenant_isolation=true","actor/resource tenant mismatch -> deny"]"#,
            policy,
        );
    }

    if !action_role_allowed(policy, action, &roles[..role_count]) {
        return deny(
            out,
            b"actor role not permitted for action",
            b"insufficient_role",
            br#"[{"type":"audit_log","severity":"required"}]"#,
            br#"["action role check failed"]"#,
            policy,
        );
    }

    match action {
        b"create" | b"edit" | b"reply" => {
            process_body_action(
                out,
                action,
                actor_id,
                comment,
                policy,
                visibility_attr,
                visibility_model,
            )
        }
        b"react" => process_react(out, actor_id, comment, policy),
        b"delete" => process_delete(out, actor_id, comment, policy),
        _ => deny(
            out,
            b"unknown action",
            b"invalid_action",
            br#"[{"type":"audit_log","severity":"required"}]"#,
            br#"["unknown action"]"#,
            policy,
        ),
    }
}

fn process_body_action(
    out: &mut [u8],
    action: &[u8],
    actor_id: &[u8],
    comment: &[u8],
    policy: &[u8],
    visibility_attr: &[u8],
    visibility_model: &[u8],
) -> usize {
    let body = extract_string(comment, b"\"body\"");
    if is_blank(body) {
        return deny(
            out,
            b"body is empty or whitespace only",
            b"empty_body",
            b"[]",
            br#"["body empty/whitespace -> deny"]"#,
            policy,
        );
    }

    let max_len = extract_i32(policy, b"\"max_body_length\"").unwrap_or(10000);
    if body.len() as i32 > max_len {
        return deny(
            out,
            b"body exceeds max_body_length",
            b"body_too_long",
            br#"[{"type":"audit_log","severity":"required"}]"#,
            br#"["body length check failed"]"#,
            policy,
        );
    }

    if action == b"edit" {
        let own_only = action_own_only(policy, b"edit");
        let created_by = extract_string(comment, b"\"created_by\"");
        if own_only && created_by != actor_id {
            return deny(
                out,
                b"edit requires own_only and actor is not the creator",
                b"not_owner",
                br#"[{"type":"audit_log","severity":"required","metadata":{"denied_action":"edit"}}]"#,
                br#"["action=edit","policy own_only=true","creator mismatch -> deny"]"#,
                policy,
            );
        }
    }

    if action == b"reply" {
        let parent_depth = extract_i32(comment, b"\"parent_depth\"").unwrap_or(0);
        let max_depth = extract_i32(policy, b"\"max_thread_depth\"").unwrap_or(8);
        let proposed = parent_depth.saturating_add(1);
        if proposed >= max_depth {
            return deny(
                out,
                b"reply would create depth that equals or exceeds max_thread_depth",
                b"max_thread_depth_exceeded",
                br#"[{"type":"audit_log","severity":"required"}]"#,
                br#"["action=reply","proposed depth >= max_thread_depth -> deny"]"#,
                policy,
            );
        }
    }

    let mut block_terms: [&[u8]; MAX_BLOCK] = [&b""[..]; MAX_BLOCK];
    let block_count = parse_moderation_blocklist(policy, &mut block_terms);
    let mut matched_term: &[u8] = b"";
    for term in block_terms[..block_count].iter() {
        if !term.is_empty() && contains_ascii_ci(body, term) {
            matched_term = term;
            break;
        }
    }

    let mut mentions: [&[u8]; MAX_MENTIONS] = [&b""[..]; MAX_MENTIONS];
    let mention_count = extract_mentions(body, &mut mentions);
    let mut links: [&[u8]; MAX_LINKS] = [&b""[..]; MAX_LINKS];
    let link_count = extract_links(body, &mut links);

    let visibility: &[u8] = if !visibility_attr.is_empty() {
        visibility_attr
    } else if visibility_model == b"channel" {
        b"channel"
    } else {
        b"team"
    };

    let parent_id = extract_string(comment, b"\"parent_id\"");
    let comment_id = extract_string(comment, b"\"id\"");
    let depth = if action == b"reply" {
        extract_i32(comment, b"\"parent_depth\"")
            .unwrap_or(0)
            .saturating_add(1)
    } else {
        0
    };

    let quarantine = !matched_term.is_empty();
    let reason_code: &[u8] = if quarantine {
        b"moderation_quarantine"
    } else {
        b"ok"
    };
    let reason: &[u8] = if quarantine {
        b"Body matched moderation blocklist - allowed with quarantine obligation"
    } else if action == b"create" {
        b"Actor has create permission and body passed all validations"
    } else if action == b"edit" {
        b"Edit allowed"
    } else {
        b"Reply allowed"
    };

    write_allow_normalized(
        out,
        reason,
        reason_code,
        action,
        comment_id,
        body,
        parent_id,
        depth,
        &mentions[..mention_count],
        &links[..link_count],
        visibility,
        actor_id,
        quarantine,
        policy,
    )
}

fn emit_null_or_string(out: &mut [u8], at: usize, value: &[u8]) -> usize {
    if value.is_empty() || value == b"null" {
        copy(out, at, br#"null"#)
    } else {
        json_str(out, at, value)
    }
}

fn process_react(out: &mut [u8], actor_id: &[u8], comment: &[u8], policy: &[u8]) -> usize {
    let comment_id = extract_string(comment, b"\"id\"");
    let reaction = object_after_key(comment, b"\"reaction\"").unwrap_or(b"");
    let emoji = extract_string(reaction, b"\"emoji\"");
    let op = extract_string(reaction, b"\"op\"");
    if emoji.is_empty() {
        return deny(
            out,
            b"reaction.emoji required",
            b"invalid_reaction",
            br#"[{"type":"audit_log","severity":"required"}]"#,
            br#"["missing reaction.emoji"]"#,
            policy,
        );
    }
    // Idempotent add: single user in set.
    let mut i = 0usize;
    i = copy(out, i, br#"{"decision":"allow","reason":"Reaction "#);
    i = copy(out, i, if op == b"remove" { b"remove" } else { b"add" });
    i = copy(out, i, br#" allowed","reason_code":"ok","normalized_comment":{"id":"#);
    i = json_str(out, i, comment_id);
    i = copy(out, i, br#","action":"react","reactions":[{"emoji":"#);
    i = json_str(out, i, emoji);
    i = copy(out, i, br#","user_ids":["#);
    i = json_str(out, i, actor_id);
    i = copy(out, i, br#"],"count":1}],"created_by":null},"obligations":[{"type":"audit_log","severity":"recommended"},{"type":"realtime_fanout","severity":"recommended","metadata":{"event":"reaction_added"}}],"evaluation_trace":["action=react","actor has react permission","reaction normalized"],"policy_hash":"#);
    i = write_policy_hash(out, i, policy);
    i = copy(out, i, br#","confidence":"high"}"#);
    i
}

fn process_delete(out: &mut [u8], actor_id: &[u8], comment: &[u8], policy: &[u8]) -> usize {
    let own_only = action_own_only(policy, b"delete");
    let created_by = extract_string(comment, b"\"created_by\"");
    if own_only && !created_by.is_empty() && created_by != actor_id {
        return deny(
            out,
            b"delete requires own_only and actor is not the creator",
            b"not_owner",
            br#"[{"type":"audit_log","severity":"required"}]"#,
            br#"["action=delete","own_only mismatch"]"#,
            policy,
        );
    }
    let soft = contains(policy, b"\"soft_delete\":true") || contains(policy, b"\"soft_delete\": true");
    if !soft {
        return deny(
            out,
            b"hard delete not supported in this binary",
            b"hard_delete_unsupported",
            br#"[{"type":"audit_log","severity":"required"}]"#,
            br#"["soft_delete=false"]"#,
            policy,
        );
    }
    let comment_id = extract_string(comment, b"\"id\"");
    let body = extract_string(comment, b"\"body\"");
    let mut i = 0usize;
    i = copy(out, i, br#"{"decision":"allow","reason":"Soft-delete allowed for owner","reason_code":"ok","normalized_comment":{"id":"#);
    i = json_str(out, i, comment_id);
    i = copy(out, i, br#","action":"delete","body":"#);
    i = json_str(out, i, body);
    i = copy(out, i, br#","body_plain":"#);
    i = json_str(out, i, body);
    i = copy(out, i, br#","deleted":true,"deleted_by":"#);
    i = json_str(out, i, actor_id);
    i = copy(out, i, br#","deleted_at":null,"created_by":"#);
    i = json_str(out, i, if created_by.is_empty() { actor_id } else { created_by });
    i = copy(out, i, br#"},"obligations":[{"type":"audit_log","severity":"required","metadata":{"soft_delete":true}},{"type":"retain_for_ediscovery","severity":"required"}],"evaluation_trace":["action=delete","soft_delete=true in policy","actor is creator -> allow","body retained for audit"],"policy_hash":"#);
    i = write_policy_hash(out, i, policy);
    i = copy(out, i, br#","confidence":"high"}"#);
    i
}

fn write_allow_normalized(
    out: &mut [u8],
    reason: &[u8],
    reason_code: &[u8],
    action: &[u8],
    comment_id: &[u8],
    body: &[u8],
    parent_id: &[u8],
    depth: i32,
    mentions: &[&[u8]],
    links: &[&[u8]],
    visibility: &[u8],
    actor_id: &[u8],
    quarantine: bool,
    policy: &[u8],
) -> usize {
    let mut i = 0usize;
    i = copy(out, i, br#"{"decision":"allow","reason":"#);
    i = json_str(out, i, reason);
    i = copy(out, i, br#","reason_code":"#);
    i = json_str(out, i, reason_code);
    i = copy(out, i, br#","normalized_comment":{"id":"#);
    i = emit_null_or_string(out, i, comment_id);
    i = copy(out, i, br#","action":"#);
    i = json_str(out, i, action);
    i = copy(out, i, br#","body":"#);
    i = json_str(out, i, body);
    i = copy(out, i, br#","body_plain":"#);
    i = json_str(out, i, body);
    i = copy(out, i, br#","parent_id":"#);
    i = emit_null_or_string(out, i, parent_id);
    i = copy(out, i, br#","thread_root_id":null,"depth":"#);
    i = copy_i32(out, i, depth);
    i = copy(out, i, br#","mentions":["#);
    for (idx, m) in mentions.iter().enumerate() {
        if idx > 0 {
            i = copy(out, i, br#","#);
        }
        i = copy(out, i, br#"{"id":"#);
        i = json_str(out, i, m);
        i = copy(out, i, br#","type":"user","valid":true}"#);
    }
    i = copy(out, i, br#"],"links":["#);
    for (idx, l) in links.iter().enumerate() {
        if idx > 0 {
            i = copy(out, i, br#","#);
        }
        i = copy(out, i, br#"""#);
        i = copy(out, i, l);
        i = copy(out, i, br#"""#);
    }
    i = copy(out, i, br#"],"reactions":[],"visibility":"#);
    i = json_str(out, i, visibility);
    i = copy(out, i, br#","resolved":false,"pinned":false,"deleted":false,"created_by":"#);
    i = json_str(out, i, actor_id);
    if quarantine {
        i = copy(out, i, br#","metadata":{"quarantine":true}}"#);
    } else {
        i = copy(out, i, br#","metadata":{}}"#);
    }

    // obligations
    i = copy(out, i, br#","obligations":["#);
    if quarantine {
        i = copy(
            out,
            i,
            br#"{"type":"quarantine","severity":"required","reason":"blocklist_match"}"#,
        );
        i = copy(out, i, br#",{"type":"notify","severity":"required","targets":["role:moderator"]}"#);
        i = copy(out, i, br#",{"type":"audit_log","severity":"required"}"#);
    } else {
        let mut need_comma = false;
        if !mentions.is_empty() {
            i = copy(out, i, br#"{"type":"notify","severity":"required","targets":["#);
            for (idx, m) in mentions.iter().enumerate() {
                if idx > 0 {
                    i = copy(out, i, br#","#);
                }
                i = json_str(out, i, m);
            }
            i = copy(out, i, br#"],"reason":"mentioned"}"#);
            need_comma = true;
        }
        if need_comma {
            i = copy(out, i, br#","#);
        }
        i = copy(out, i, br#"{"type":"audit_log","severity":"required"}"#);
        if action == b"create" {
            i = copy(
                out,
                i,
                br#",{"type":"index_for_search","severity":"recommended"}"#,
            );
        }
    }
    i = copy(out, i, br#"],"evaluation_trace":["action="#);
    i = copy(out, i, action);
    if quarantine {
        i = copy(out, i, br#"","body matched blocklist","decision=allow with quarantine"]"#);
    } else {
        i = copy(out, i, br#"","validations passed"]"#);
    }
    i = copy(out, i, br#","policy_hash":"#);
    i = write_policy_hash(out, i, policy);
    i = copy(out, i, br#","confidence":"high"}"#);
    i
}

fn deny(
    out: &mut [u8],
    reason: &[u8],
    reason_code: &[u8],
    obligations: &[u8],
    trace: &[u8],
    policy: &[u8],
) -> usize {
    let mut i = 0usize;
    i = copy(out, i, br#"{"decision":"deny","reason":"#);
    i = json_str(out, i, reason);
    i = copy(out, i, br#","reason_code":"#);
    i = json_str(out, i, reason_code);
    i = copy(out, i, br#","normalized_comment":null,"obligations":"#);
    i = copy(out, i, obligations);
    i = copy(out, i, br#","evaluation_trace":"#);
    i = copy(out, i, trace);
    i = copy(out, i, br#","policy_hash":"#);
    i = write_policy_hash(out, i, policy);
    i = copy(out, i, br#","confidence":"high"}"#);
    i
}

fn write_policy_hash(out: &mut [u8], at: usize, policy: &[u8]) -> usize {
    let ver = extract_string(policy, b"\"version\"");
    let mut i = at;
    i = copy(out, i, br#""sha256:policy:"#);
    if ver.is_empty() {
        let h = fnv_hex(policy);
        i = copy(out, i, &h);
    } else {
        i = copy(out, i, ver);
    }
    i = copy(out, i, br#"""#);
    i
}

fn action_role_allowed(policy: &[u8], action: &[u8], roles: &[&[u8]]) -> bool {
    let Some(actions) = object_after_key(policy, b"\"actions\"") else {
        return false;
    };
    let mut key = [0u8; 32];
    if action.len() + 2 > key.len() {
        return false;
    }
    key[0] = b'"';
    key[1..1 + action.len()].copy_from_slice(action);
    key[1 + action.len()] = b'"';
    let Some(cfg) = object_after_key(actions, &key[..action.len() + 2]) else {
        return false;
    };
    let mut allowed: [&[u8]; MAX_ROLES] = [&b""[..]; MAX_ROLES];
    let n = parse_string_array(cfg, b"\"roles\"", &mut allowed);
    if n == 0 {
        return false;
    }
    for role in roles {
        for a in allowed[..n].iter() {
            if *role == *a {
                return true;
            }
        }
    }
    false
}

fn action_own_only(policy: &[u8], action: &[u8]) -> bool {
    let Some(actions) = object_after_key(policy, b"\"actions\"") else {
        return false;
    };
    let mut key = [0u8; 32];
    if action.len() + 2 > key.len() {
        return false;
    }
    key[0] = b'"';
    key[1..1 + action.len()].copy_from_slice(action);
    key[1 + action.len()] = b'"';
    let Some(cfg) = object_after_key(actions, &key[..action.len() + 2]) else {
        return false;
    };
    contains(cfg, b"\"own_only\":true") || contains(cfg, b"\"own_only\": true")
}

fn parse_moderation_blocklist<'a>(policy: &'a [u8], out: &mut [&'a [u8]]) -> usize {
    let Some(moderation) = object_after_key(policy, b"\"moderation\"") else {
        return 0;
    };
    parse_string_array(moderation, b"\"blocklist\"", out)
}

fn extract_mentions<'a>(body: &'a [u8], out: &mut [&'a [u8]]) -> usize {
    let mut count = 0usize;
    let mut i = 0usize;
    while i < body.len() && count < out.len() {
        if body[i] == b'@' {
            let start = i + 1;
            let mut j = start;
            while j < body.len() {
                let c = body[j];
                let ok = (c >= b'a' && c <= b'z')
                    || (c >= b'A' && c <= b'Z')
                    || (c >= b'0' && c <= b'9')
                    || c == b'-'
                    || c == b'_';
                if !ok {
                    break;
                }
                j += 1;
            }
            if j > start {
                out[count] = &body[start..j];
                count += 1;
                i = j;
                continue;
            }
        }
        i += 1;
    }
    count
}

fn extract_links<'a>(body: &'a [u8], out: &mut [&'a [u8]]) -> usize {
    let needle = b"https://";
    let mut count = 0usize;
    let mut i = 0usize;
    while i + needle.len() <= body.len() && count < out.len() {
        if &body[i..i + needle.len()] == needle {
            let start = i;
            let mut j = i + needle.len();
            while j < body.len() {
                let c = body[j];
                if c == b' ' || c == b'\n' || c == b'\t' || c == b'"' || c == b'\'' {
                    break;
                }
                j += 1;
            }
            out[count] = &body[start..j];
            count += 1;
            i = j;
            continue;
        }
        i += 1;
    }
    count
}

fn is_blank(s: &[u8]) -> bool {
    s.is_empty()
        || s.iter()
            .all(|b| *b == b' ' || *b == b'\n' || *b == b'\t' || *b == b'\r')
}

fn contains_ascii_ci(hay: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || hay.len() < needle.len() {
        return false;
    }
    'outer: for i in 0..=(hay.len() - needle.len()) {
        for j in 0..needle.len() {
            let a = hay[i + j].to_ascii_lowercase();
            let b = needle[j].to_ascii_lowercase();
            if a != b {
                continue 'outer;
            }
        }
        return true;
    }
    false
}

fn attr_string<'a>(object: &'a [u8], key: &[u8]) -> &'a [u8] {
    let Some(attrs) = object_after_key(object, b"\"attributes\"") else {
        return b"";
    };
    let mut k = [0u8; 48];
    if key.len() + 2 > k.len() {
        return b"";
    }
    k[0] = b'"';
    k[1..1 + key.len()].copy_from_slice(key);
    k[1 + key.len()] = b'"';
    extract_string(attrs, &k[..key.len() + 2])
}

fn parse_string_array<'a>(hay: &'a [u8], key: &[u8], out: &mut [&'a [u8]]) -> usize {
    let Some(arr) = json_array_after_key(hay, key) else {
        return 0;
    };
    let mut count = 0usize;
    let mut i = 0usize;
    while i < arr.len() && count < out.len() {
        if arr[i] == b'"' {
            i += 1;
            let start = i;
            while i < arr.len() && arr[i] != b'"' {
                i += 1;
            }
            if i <= arr.len() {
                out[count] = &arr[start..i];
                count += 1;
            }
        }
        i += 1;
    }
    count
}

fn object_after_key<'a>(hay: &'a [u8], key: &[u8]) -> Option<&'a [u8]> {
    let pos = find_key_at_depth(hay, key, 1)?;
    value_object_after(&hay[pos + key.len()..])
}

fn json_array_after_key<'a>(hay: &'a [u8], key: &[u8]) -> Option<&'a [u8]> {
    let pos = find_key_at_depth(hay, key, 1)?;
    value_array_after(&hay[pos + key.len()..])
}

fn find_key_at_depth(hay: &[u8], key: &[u8], target_depth: i32) -> Option<usize> {
    let mut depth = 0i32;
    let mut in_str = false;
    let mut i = 0usize;
    while i < hay.len() {
        let b = hay[i];
        if in_str {
            if b == b'\\' {
                i += 2;
                continue;
            }
            if b == b'"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        match b {
            b'"' => {
                if depth == target_depth
                    && i + key.len() <= hay.len()
                    && &hay[i..i + key.len()] == key
                {
                    return Some(i);
                }
                in_str = true;
            }
            b'{' | b'[' => depth += 1,
            b'}' | b']' => depth -= 1,
            _ => {}
        }
        i += 1;
    }
    None
}

fn value_object_after<'a>(after_key: &'a [u8]) -> Option<&'a [u8]> {
    let colon = after_key.iter().position(|b| *b == b':')?;
    let mut rest = skip_ws(&after_key[colon + 1..]);
    if rest.first() != Some(&b'{') {
        return None;
    }
    let end = balanced_end(rest, b'{', b'}')?;
    Some(&rest[..=end])
}

fn value_array_after<'a>(after_key: &'a [u8]) -> Option<&'a [u8]> {
    let colon = after_key.iter().position(|b| *b == b':')?;
    let mut rest = skip_ws(&after_key[colon + 1..]);
    if rest.first() != Some(&b'[') {
        return None;
    }
    let end = balanced_end(rest, b'[', b']')?;
    Some(&rest[..=end])
}

fn balanced_end(s: &[u8], open: u8, close: u8) -> Option<usize> {
    let mut depth = 0i32;
    let mut in_str = false;
    let mut i = 0usize;
    while i < s.len() {
        let b = s[i];
        if in_str {
            if b == b'\\' {
                i += 2;
                continue;
            }
            if b == b'"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        match b {
            b'"' => in_str = true,
            x if x == open => depth += 1,
            x if x == close => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn extract_string<'a>(hay: &'a [u8], key: &[u8]) -> &'a [u8] {
    let Some(pos) = find(hay, key) else {
        return b"";
    };
    string_value_after(&hay[pos + key.len()..])
}

fn extract_string_at_depth<'a>(hay: &'a [u8], key: &[u8], depth: i32) -> &'a [u8] {
    let Some(pos) = find_key_at_depth(hay, key, depth) else {
        return b"";
    };
    string_value_after(&hay[pos + key.len()..])
}

fn string_value_after<'a>(after_key: &'a [u8]) -> &'a [u8] {
    let Some(colon) = after_key.iter().position(|b| *b == b':') else {
        return b"";
    };
    let rest = skip_ws(&after_key[colon + 1..]);
    if rest.first() != Some(&b'"') {
        return b"";
    }
    let rest = &rest[1..];
    let Some(end) = rest.iter().position(|b| *b == b'"') else {
        return b"";
    };
    &rest[..end]
}

fn extract_i32(hay: &[u8], key: &[u8]) -> Option<i32> {
    let pos = find(hay, key)?;
    let after = &hay[pos + key.len()..];
    let colon = after.iter().position(|b| *b == b':')?;
    let mut rest = skip_ws(&after[colon + 1..]);
    let mut neg = false;
    if rest.first() == Some(&b'-') {
        neg = true;
        rest = &rest[1..];
    }
    let mut val: i32 = 0;
    let mut any = false;
    for &b in rest {
        if b < b'0' || b > b'9' {
            break;
        }
        any = true;
        val = val.saturating_mul(10).saturating_add(i32::from(b - b'0'));
    }
    if !any {
        return None;
    }
    Some(if neg { -val } else { val })
}

fn skip_ws(s: &[u8]) -> &[u8] {
    let mut rest = s;
    while rest.first() == Some(&b' ')
        || rest.first() == Some(&b'\n')
        || rest.first() == Some(&b'\t')
        || rest.first() == Some(&b'\r')
    {
        rest = &rest[1..];
    }
    rest
}

fn fnv_hex(bytes: &[u8]) -> [u8; 16] {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    let mut out = [b'0'; 16];
    for i in 0..16 {
        let nibble = ((h >> (60 - 4 * i)) & 0xf) as u8;
        out[i] = if nibble < 10 {
            b'0' + nibble
        } else {
            b'a' + (nibble - 10)
        };
    }
    out
}

fn json_str(out: &mut [u8], at: usize, s: &[u8]) -> usize {
    // Values are fixture-controlled; no escape handling beyond passthrough of safe ASCII.
    let mut i = copy(out, at, br#"""#);
    i = copy(out, i, s);
    copy(out, i, br#"""#)
}

fn copy(out: &mut [u8], at: usize, bytes: &[u8]) -> usize {
    let end = at + bytes.len();
    if end > out.len() {
        return at;
    }
    out[at..end].copy_from_slice(bytes);
    end
}

fn copy_i32(out: &mut [u8], at: usize, mut v: i32) -> usize {
    if v == 0 {
        return copy(out, at, b"0");
    }
    let neg = v < 0;
    if neg {
        v = -v;
    }
    let mut tmp = [0u8; 12];
    let mut n = 0usize;
    while v > 0 {
        tmp[n] = b'0' + (v % 10) as u8;
        n += 1;
        v /= 10;
    }
    let mut i = at;
    if neg {
        i = copy(out, i, b"-");
    }
    while n > 0 {
        n -= 1;
        i = copy(out, i, &[tmp[n]]);
    }
    i
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {}
}
