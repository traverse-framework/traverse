;; Spec 138 interruption conformance fixture (Decision 111, #1642).
;; `model_execute` loops forever, so a host proves that cancellation and
;; deadlines interrupt a running inference mid-run (FR-029). Its fuel and time
;; ceilings are large enough that only an interruption ends a call.
(module
  (memory (export "memory") 1)
  (func (export "model_execute") (param i32 i32 i32 i32) (result i32)
    (loop $spin (br $spin))
    (i32.const 0)))
