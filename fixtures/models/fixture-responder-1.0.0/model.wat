(module
  (memory (export "memory") 2)
  (data (i32.const 40000) "hi")
  (data (i32.const 40010) "hi there")
  (data (i32.const 40030) "hmm")
  (func (export "model_execute")
    (param $in_ptr i32) (param $in_len i32) (param $out_ptr i32) (param $out_cap i32) (result i32)
    (local $payload_len i32)
    (local $payload_ptr i32)
    (local $i i32)
    (local $j i32)
    (local $matched i32)
    (local $resp_ptr i32)
    (local $resp_len i32)

    (if (i32.lt_u (local.get $in_len) (i32.const 12))
      (then (return (i32.const -1))))

    (local.set $payload_len (i32.load offset=8 (local.get $in_ptr)))
    (local.set $payload_ptr (i32.add (local.get $in_ptr) (i32.const 12)))

    (if (i32.gt_u (i32.add (i32.const 12) (local.get $payload_len)) (local.get $in_len))
      (then (return (i32.const -1))))

    (local.set $matched (i32.const 0))
    (local.set $i (i32.const 0))
    (block $search_done
      (loop $search
        (br_if $search_done (i32.gt_u (i32.add (local.get $i) (i32.const 2)) (local.get $payload_len)))
        (if (i32.and
              (i32.eq (i32.load8_u (i32.add (local.get $payload_ptr) (local.get $i))) (i32.const 104))
              (i32.eq (i32.load8_u (i32.add (local.get $payload_ptr) (i32.add (local.get $i) (i32.const 1)))) (i32.const 105)))
          (then
            (local.set $matched (i32.const 1))
            (br $search_done)))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $search)))

    (if (i32.eq (local.get $matched) (i32.const 1))
      (then
        (local.set $resp_ptr (i32.const 40010))
        (local.set $resp_len (i32.const 8)))
      (else
        (local.set $resp_ptr (i32.const 40030))
        (local.set $resp_len (i32.const 3))))

    (if (i32.gt_u (i32.add (i32.const 12) (local.get $resp_len)) (local.get $out_cap))
      (then (return (i32.const -1))))

    (i32.store16 offset=0 (local.get $out_ptr) (i32.const 1))
    (i32.store8 offset=2 (local.get $out_ptr) (i32.const 4))
    (i32.store8 offset=3 (local.get $out_ptr) (i32.const 1))
    (i32.store offset=4 (local.get $out_ptr) (local.get $resp_len))
    (i32.store offset=8 (local.get $out_ptr) (local.get $resp_len))

    (local.set $j (i32.const 0))
    (block $copy_done
      (loop $copy
        (br_if $copy_done (i32.ge_u (local.get $j) (local.get $resp_len)))
        (i32.store8
          (i32.add (i32.add (local.get $out_ptr) (i32.const 12)) (local.get $j))
          (i32.load8_u (i32.add (local.get $resp_ptr) (local.get $j))))
        (local.set $j (i32.add (local.get $j) (i32.const 1)))
        (br $copy)))

    (i32.add (i32.const 12) (local.get $resp_len))
  )
)
