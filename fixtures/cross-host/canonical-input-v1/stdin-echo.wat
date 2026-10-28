;; Spec 1722 FR-006 conformance probe: returns its stdin bytes verbatim as the
;; JSON string `{"stdin":"<bytes>"}`, so a host can compare the exact bytes a
;; guest read (and their SHA-256) after its own JSON round trip of the output.
;; Canonical input already escapes control characters, so only `"` and `\`
;; need escaping here. Reads at most 61,440 bytes of stdin.
(module
  (import "wasi_snapshot_preview1" "fd_read"
    (func $fd_read (param i32 i32 i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "fd_write"
    (func $fd_write (param i32 i32 i32 i32) (result i32)))
  (memory (export "memory") 4)
  ;; Output starts at 0x20000 with this 10-byte prefix already in place.
  (data (i32.const 0x20000) "{\"stdin\":\"")
  (func (export "_start")
    (local $len i32)
    (local $read i32)
    (local $index i32)
    (local $out i32)
    (local $byte i32)
    ;; Read all of stdin into 0x1000.. with an iovec at 0 and nread at 8.
    (block $done
      (loop $more
        (i32.store (i32.const 0) (i32.add (i32.const 0x1000) (local.get $len)))
        (i32.store (i32.const 4) (i32.sub (i32.const 0xF000) (local.get $len)))
        (br_if $done
          (call $fd_read (i32.const 0) (i32.const 0) (i32.const 1) (i32.const 8)))
        (local.set $read (i32.load (i32.const 8)))
        (br_if $done (i32.eqz (local.get $read)))
        (local.set $len (i32.add (local.get $len) (local.get $read)))
        (br $more)))
    ;; Copy it after the prefix, escaping `"` (34) and `\` (92).
    (local.set $out (i32.const 0x2000A))
    (block $copied
      (loop $copy
        (br_if $copied (i32.ge_u (local.get $index) (local.get $len)))
        (local.set $byte
          (i32.load8_u (i32.add (i32.const 0x1000) (local.get $index))))
        (if (i32.or (i32.eq (local.get $byte) (i32.const 34))
                    (i32.eq (local.get $byte) (i32.const 92)))
          (then
            (i32.store8 (local.get $out) (i32.const 92))
            (local.set $out (i32.add (local.get $out) (i32.const 1)))))
        (i32.store8 (local.get $out) (local.get $byte))
        (local.set $out (i32.add (local.get $out) (i32.const 1)))
        (local.set $index (i32.add (local.get $index) (i32.const 1)))
        (br $copy)))
    (i32.store8 (local.get $out) (i32.const 34))
    (i32.store8 (i32.add (local.get $out) (i32.const 1)) (i32.const 125))
    ;; Write [0x20000, out + 2) to stdout with an iovec at 16 and nwritten at 24.
    (i32.store (i32.const 16) (i32.const 0x20000))
    (i32.store (i32.const 20)
      (i32.sub (i32.add (local.get $out) (i32.const 2)) (i32.const 0x20000)))
    (drop (call $fd_write (i32.const 1) (i32.const 16) (i32.const 1) (i32.const 24)))))
