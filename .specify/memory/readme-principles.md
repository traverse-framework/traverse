# README & Documentation Principles

Established in brainstorm session, 2026-06-25. These principles apply to every
release update of README.md and the key docs listed below.

---

## Primary Audience

**The developer who has never heard of UMA, WASM, or "governed capabilities."**

This is the person who landed from a GitHub search, a blog post, or a tweet.
They don't know the vocabulary. They don't care about the architecture yet.
They want to know in 30 seconds: does this solve my problem?

Secondary audiences (integrators, contributors, AI agents) are served below
the fold — they are NOT the primary audience for the opening section.

---

## First Screen Rules (everything visible before scrolling)

1. **Problem-first hook.** Lead with pain the developer already feels, not with
   the solution's name or architecture. They must nod before they read the answer.

2. **Zero internal jargon.** The words "governed", "capability", "contract",
   "spec-aligned", "app-consumable", "speckit", and "youaskm3" must not appear
   in the first screen. Use plain English equivalents.

3. **One honest sentence about what Traverse is.** After the hook, one sentence
   that says what the tool actually does in terms a newcomer understands.

4. **Show it working.** A code snippet or terminal output that a developer can
   skim in 10 seconds and think "I get what this does." It must work exactly as
   written at the time of release — no aspirational commands.

---

## The Hook

Lead with duplication pain — it's universally felt and requires no setup:

> Your business logic runs in the browser, on your server, and in a cloud
> function. They drift. You maintain three versions of the same behavior.
> Traverse keeps it in one contract and runs it anywhere.

Update the specific environments ("browser, server, cloud function") to match
the platforms Traverse actually supports at the current release, but preserve
the structure: pain (duplication + drift) → resolution (one contract, runs anywhere).

---

## Quickstart Snippet Rules

- Show the **expedition run** as the first taste — it is real, it works from
  a fresh clone, and it demonstrates the core loop (register → run → trace).
- `traverse-cli app new` is the "now build your own" step — it comes *after*
  the first taste, not before.
- The React demo is the "see the full picture" link — link to quickstart.md,
  don't replicate the two-terminal setup in the README.
- **Never show a command that doesn't work at the current release version.**
  If a command changed, update it.

---

## What Can I Build Section

Keep this section. It is the most useful thing for a new developer after the
hook and quickstart. Rules:

- List only what works today, not future directions.
- Each item: one sentence what you build, one sentence what Traverse owns.
- Link to the relevant doc, not to governance artifacts.

---

## Structure (top to bottom)

1. Badge row (CI, coverage, version, license)
2. Problem-first hook (2–3 sentences max)
3. What it is (1 sentence)
4. Quickstart snippet (works as written)
5. What can I build (3–5 bullets, today only)
6. Documentation map (goal → doc table)
7. Architecture (crates table)
8. Contributing
9. --- fold ---
10. Governance & approved specs (move here, preserve content)
11. For Agents section (move here, preserve content)
12. Related Work / License

---

## UMA Positioning

After the hook and the one-sentence "what it is", add exactly one line linking
to UMA — then move on:

> Traverse is the working implementation of [Universal Microservices Architecture](https://www.universalmicroservices.com/).

The full UMA table ("What it is / Business capabilities / Portability...") moves
below the fold into the Related Work section. New developers never see it on
first load; UMA readers find it when they scroll.

## Docs Scope

The skill touches exactly three files per release:
1. `README.md` — storefront
2. `quickstart.md` — first experience
3. `docs/what-can-i-build.md` — "is this for me" page

`docs/getting-started.md` is the natural next file to add to this scope once
the three above are stable.

## What to Preserve Exactly

- The "For Agents" section content — only move it below the fold.
- The approved specs table — only move it below the fold.
- The UMA/Built on UMA section — keep, but move below the fold.
- All doc links — keep them, just reorganize into the documentation map.
- Badges — keep, update version number to current release.

---

## Tone

- Write for a senior developer who is skeptical and time-poor.
- Earn every claim. Don't say "powerful" or "seamless".
- Short sentences. No passive voice.
- If something isn't proven yet, don't write it in the opening section.
