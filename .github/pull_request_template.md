## What

<!-- One-line summary of the change. -->

## Why

<!-- Link to issue or describe the motivation. -->

## How

<!-- Key implementation decisions. Skip obvious stuff. -->

## Testing

- [ ] `make ci` passes locally (fmt + clippy + test + audit)
- [ ] New behavior has tests
- [ ] Hot-path changes have criterion benchmarks

## Checklist

- [ ] PR description explains *why*, not just *what*
- [ ] No new `unwrap()`/`expect()` in production code
- [ ] No new allocations in the simulation hot loop
- [ ] Crate boundaries respected (pirx-core never imports from pirx-adapters)
- [ ] New dependencies justified (not "it's popular" — what does it replace?)
