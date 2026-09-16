(module
  (memory (export "memory") 2)
  (func (export "model_execute")
    (param $in_ptr i32) (param $in_len i32) (param $out_ptr i32) (param $out_cap i32) (result i32)
    (local $x0 f32) (local $x1 f32) (local $x2 f32) (local $x3 f32) (local $score f32) (local $label f32)
    (if (i32.lt_u (local.get $in_len) (i32.const 28))
      (then (return (i32.const -1))))
    (if (i32.lt_u (local.get $out_cap) (i32.const 20))
      (then (return (i32.const -1))))
    (local.set $x0 (f32.load offset=12 (local.get $in_ptr)))
    (local.set $x1 (f32.load offset=16 (local.get $in_ptr)))
    (local.set $x2 (f32.load offset=20 (local.get $in_ptr)))
    (local.set $x3 (f32.load offset=24 (local.get $in_ptr)))
    (local.set $score
      (f32.sub
        (f32.add
          (f32.add
            (f32.mul (local.get $x0) (f32.const 0.5))
            (f32.mul (local.get $x1) (f32.const -0.25)))
          (f32.add
            (f32.mul (local.get $x2) (f32.const 1.0))
            (f32.mul (local.get $x3) (f32.const 0.75))))
        (f32.const 0.5)))
    (local.set $label
      (select (f32.const 1.0) (f32.const 0.0) (f32.ge (local.get $score) (f32.const 0.0))))
    (i32.store16 offset=0 (local.get $out_ptr) (i32.const 1))
    (i32.store8 offset=2 (local.get $out_ptr) (i32.const 3))
    (i32.store8 offset=3 (local.get $out_ptr) (i32.const 1))
    (i32.store offset=4 (local.get $out_ptr) (i32.const 2))
    (i32.store offset=8 (local.get $out_ptr) (i32.const 8))
    (f32.store offset=12 (local.get $out_ptr) (local.get $score))
    (f32.store offset=16 (local.get $out_ptr) (local.get $label))
    (i32.const 20)
  )
)
