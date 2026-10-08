;; Spec 138 guest ABI v3 conformance fixture (Decision 110, #1626).
;;
;; `model_prepare` builds a 256-byte key table at offset 1024 and sets one
;; exported mutable global of each numeric type. `model_execute` fails closed
;; (-1) unless that prepared state is present, XORs the input with the key
;; table, and appends one byte: the call count. It also overwrites the key
;; table and increments the count, so any memory or global state carried from
;; one call into the next changes the output. Every call that starts from the
;; pristine post-prepare state returns `input XOR key || 0x01`, whether the
;; host prepared it fresh or restored a snapshot.
(module
  (memory (export "memory") 1)
  (global $heap (export "heap") (mut i32) (i32.const 4096))
  (global $calls (export "calls") (mut i32) (i32.const 0))
  (global $prepared (export "prepared") (mut i64) (i64.const 0))
  (global $scale (export "scale") (mut f32) (f32.const 0))
  (global $offset (export "offset") (mut f64) (f64.const 0))
  (func (export "model_prepare") (result i32)
    (local $i i32)
    (block $done
      (loop $fill
        (br_if $done (i32.ge_u (local.get $i) (i32.const 256)))
        (i32.store8
          (i32.add (i32.const 1024) (local.get $i))
          (i32.add (i32.mul (local.get $i) (i32.const 37)) (i32.const 11)))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $fill)))
    (global.set $prepared (i64.const 0x0123456789abcdef))
    (global.set $scale (f32.const 1.5))
    (global.set $offset (f64.const -2.25))
    (i32.const 0))
  (func (export "model_alloc") (param $len i32) (result i32)
    (local $ptr i32) (local $end i32) (local $missing i32)
    (local.set $ptr (global.get $heap))
    (local.set $end
      (i32.and (i32.add (i32.add (local.get $ptr) (local.get $len)) (i32.const 7)) (i32.const -8)))
    (local.set $missing
      (i32.sub (i32.shr_u (i32.add (local.get $end) (i32.const 65535)) (i32.const 16)) (memory.size)))
    (if (i32.gt_s (local.get $missing) (i32.const 0))
      (then
        (if (i32.eq (memory.grow (local.get $missing)) (i32.const -1))
          (then (return (i32.const 0))))))
    (global.set $heap (local.get $end))
    (local.get $ptr))
  (func (export "model_execute")
    (param $in_ptr i32) (param $in_len i32) (param $out_ptr i32) (param $out_cap i32) (result i32)
    (local $i i32) (local $key i32) (local $byte i32)
    (if (i32.or
          (i64.ne (global.get $prepared) (i64.const 0x0123456789abcdef))
          (i32.or
            (f32.ne (global.get $scale) (f32.const 1.5))
            (f64.ne (global.get $offset) (f64.const -2.25))))
      (then (return (i32.const -1))))
    (if (i32.gt_u (i32.add (local.get $in_len) (i32.const 1)) (local.get $out_cap))
      (then (return (i32.const -1))))
    (global.set $calls (i32.add (global.get $calls) (i32.const 1)))
    (block $done
      (loop $xor
        (br_if $done (i32.ge_u (local.get $i) (local.get $in_len)))
        (local.set $key (i32.add (i32.const 1024) (i32.and (local.get $i) (i32.const 255))))
        (local.set $byte
          (i32.xor
            (i32.load8_u (i32.add (local.get $in_ptr) (local.get $i)))
            (i32.load8_u (local.get $key))))
        (i32.store8 (i32.add (local.get $out_ptr) (local.get $i)) (local.get $byte))
        (i32.store8 (local.get $key) (local.get $byte))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $xor)))
    (i32.store8 (i32.add (local.get $out_ptr) (local.get $in_len)) (global.get $calls))
    (i32.add (local.get $in_len) (i32.const 1))))
