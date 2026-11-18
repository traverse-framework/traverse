// Decision 135 rule 5 (#1891, Spec 138 FR-080): production manifests make no
// signing claims. Rights fields cover rights only; signing status comes only
// from verification against TRAVERSE_MODEL_SIGNING_KEYS. Test-only fixtures
// keep their truthful "test-only key" wording, and a superseded production
// package keeps its already-signed bytes until it is removed.
const SIGNING_WORDING = /\b(?:sign(?:ed|ing|ature|atures|s|er)?|keys?|key[_ -]?ids?|ed25519)\b/i;

export function signingClaimsIn(rights) {
  return Object.entries(rights ?? {})
    .filter(([, value]) => typeof value === "string" && SIGNING_WORDING.test(value))
    .map(([field]) => field);
}

export function assertNoSigningClaims(fixture, rights) {
  if (!fixture.production_signed || fixture.superseded_by) {
    return;
  }
  const fields = signingClaimsIn(rights);
  if (fields.length > 0) {
    throw new Error(
      `${fixture.dir}: production rights text must not mention signing or keys (Decision 135); fix ${fields.map((f) => `rights.${f}`).join(", ")}`,
    );
  }
}
