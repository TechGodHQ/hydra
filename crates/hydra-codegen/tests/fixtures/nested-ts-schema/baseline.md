# COD-521 baseline evidence

Before the precedence-aware renderer was applied, the fixture was generated from Hydra `origin/main` at `b210ff55147dbf5667b2c098c833a94a96126fba` and compiled with the pinned TypeScript 5.9.2 compiler.

Command:

```text
node examples/ts-client/node_modules/typescript/bin/tsc --target ES2022 --module NodeNext --moduleResolution NodeNext --strict --skipLibCheck --lib ES2022,DOM --noEmit type-tests.ts
```

Observed result: exit code `2` with four unused expectation failures, proving that the ungrouped baseline accepted invalid values:

```text
type-tests.ts(21,7): error TS2578: Unused '@ts-expect-error' directive.
type-tests.ts(24,5): error TS2578: Unused '@ts-expect-error' directive.
type-tests.ts(26,5): error TS2578: Unused '@ts-expect-error' directive.
type-tests.ts(29,7): error TS2578: Unused '@ts-expect-error' directive.
```

The final `npm run test:nested-schema` command regenerates the same declaration with the current renderer, performs two byte-identical writes, runs `hydra check`, and compiles the fixture again. It is the positive proof that the same invalid assignments are rejected after the fix.
