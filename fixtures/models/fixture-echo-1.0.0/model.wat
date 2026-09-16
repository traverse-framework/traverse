(module
  (memory (export "memory") 2)
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
    (local.get $n)
  )
)
