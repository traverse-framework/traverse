# UCI Optical Recognition of Handwritten Digits

Vendored unchanged from the UCI Machine Learning Repository for the
Spec 138 trained exact-ref model (Decision 102, #1461).

- **Dataset:** Optical Recognition of Handwritten Digits
- **Creators:** E. Alpaydin, C. Kaynak (Bogazici University), 1998
- **Source:** https://archive.ics.uci.edu/dataset/80/optical+recognition+of+handwritten+digits
- **DOI:** https://doi.org/10.24432/C50P49
- **License:** Creative Commons Attribution 4.0 International (CC BY 4.0),
  https://creativecommons.org/licenses/by/4.0/legalcode
- **Changes:** none. `optdigits.tra` (3,823 training rows), `optdigits.tes`
  (1,797 test rows), and `optdigits.names` (the dataset description) are
  byte-identical to the archive. Their SHA-256 digests are pinned in
  `SHA256SUMS` and asserted by the trainer's tests.

Each row is 64 comma-separated integers in `0..=16` (an 8×8 grid of 4×4
block pixel counts) followed by the class label `0..=9`.

Citation: Alpaydin, E. & Kaynak, C. (1998). Optical Recognition of
Handwritten Digits [Dataset]. UCI Machine Learning Repository.
https://doi.org/10.24432/C50P49
