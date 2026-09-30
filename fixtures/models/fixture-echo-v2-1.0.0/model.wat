;; Spec 138 guest ABI v2 conformance fixture (Decision 105, #1588).
;; Same echo semantics as fixture-echo-1.0.0, but the host obtains the input
;; and output buffers from the guest's bump allocator (`model_alloc`) instead
;; of writing at a fixed offset. `model_alloc` grows memory as needed and
;; returns 0 when it cannot (the host rejects a 0 pointer).
(module
  (memory (export "memory") 1)
  (global $heap (mut i32) (i32.const 1024))
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
    (local $i i32)
    (local $n i32)
    (local.set $n (local.get $in_len))
    (if (i32.gt_u (local.get $n) (local.get $out_cap))
      (then (local.set $n (local.get $out_cap))))
    (block $done
      (loop $copy
        (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
        (i32.store8
          (i32.add (local.get $out_ptr) (local.get $i))
          (i32.load8_u (i32.add (local.get $in_ptr) (local.get $i))))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $copy)))
    (local.get $n)))
