---
name: ha-test-strategy
description: "Hope-native test strategy for features, fixes, and refactors: select test-first, regression-first, characterization, integration, E2E, or manual evidence according to risk and repository rules."
paths: ["*.rs", "*.ts", "*.tsx", "*.js", "*.jsx", "*.py", "*.go", "*.java", "*.kt", "*.swift", "*.c", "*.cpp", "*.h", "*.rb", "*.php", "*.sh"]
---

# Hope Test Strategy

Choose tests that reduce uncertainty about the requested behavior. Test-first is
a valuable technique, not an unconditional law.

## Precedence

Follow the user's request and repository instructions. Do not run broad suites,
install dependencies, or rewrite test infrastructure when the project forbids or
does not require it.

## Select The Strategy

### Test-first

Prefer a failing test before implementation when the new contract is clear, the
test seam is stable, and observing the failure proves the test is meaningful.

### Regression-first

For a bug, reproduce the original failure in the narrowest credible automated
test before or alongside the fix. Confirm it fails for the right reason, then
passes after the root-cause change.

### Characterization-first

For legacy, poorly documented, or risky refactors, capture current intentional
behavior before changing structure. Do not freeze a known bug as desired
behavior.

### Implementation-first with immediate coverage

Reasonable for exploratory seams, generated code, mechanical migrations, or UI
work where the correct test boundary becomes clear only after a small reversible
implementation. Add the relevant proof before claiming completion.

### No new automated test

Reasonable for pure docs, trivial metadata, generated outputs, or low-risk
mechanical changes when existing checks directly cover the risk. Explain the
decision rather than adding a meaningless test.

## Choose The Layer

- Unit: pure logic, state transition, parser, policy, or failure classification.
- Integration: persistence, adapter, protocol, or cross-module contract.
- E2E: a user-critical path that only becomes true across the full stack.
- Manual smoke: visual layout, OS integration, credentials, or external state
  that cannot be honestly simulated.

Use the lowest layer that proves the contract. Add a higher layer only for a
boundary the lower layer cannot cover.

## Test Quality

- Assert observable behavior, not incidental implementation details.
- Keep fixtures deterministic and failures diagnostic.
- Cover the demonstrated edge, including cancellation, retry, stale state, or
  partial failure when relevant.
- Avoid mocks that erase the boundary being tested.
- Do not weaken existing assertions merely to make a change pass.

## Execute Efficiently

1. Run the narrowest new or failing test.
2. Run the nearest affected suite when justified.
3. Use `ha-verify` to decide whether broader gates are needed at closeout.

If a test is flaky or environment-blocked, investigate the cause. Do not rerun
until green and call that proof.

## Smoke Prompts

- "Choose and add the right regression test for this bug."
- "Refactor this legacy parser without freezing the broken behavior."
- "Decide whether this UI change needs unit, E2E, or manual evidence."
