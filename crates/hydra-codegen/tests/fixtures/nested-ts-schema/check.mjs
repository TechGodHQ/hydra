import { cpSync, existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { tmpdir } from "node:os";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const fixtureDir = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(fixtureDir, "../../../../../");
const tempBase = process.env.HYDRA_TEST_TMP
  ?? process.env.TMPDIR
  ?? (existsSync("/opt/data/tmp") ? "/opt/data/tmp" : tmpdir());
const tempRoot = mkdtempSync(join(tempBase, "hydra-nested-ts-"));
const fixtureTemp = join(tempRoot, "fixture");
const generatedDir = join(fixtureTemp, "generated");
const cargo = process.env.CARGO ?? "cargo";
const tsc = join(repoRoot, "examples", "ts-client", "node_modules", "typescript", "bin", "tsc");

function run(command, args) {
  const result = spawnSync(command, args, {
    cwd: fixtureTemp,
    encoding: "utf8",
    stdio: "inherit",
  });
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    throw new Error(`${command} ${args.join(" ")} exited with ${result.status}`);
  }
}

try {
  if (!existsSync(tsc)) {
    throw new Error(`TypeScript is not installed at ${tsc}; run npm ci --prefix examples/ts-client first`);
  }
  cpSync(join(fixtureDir, "api"), join(fixtureTemp, "api"), { recursive: true });
  cpSync(join(fixtureDir, "hydra.yaml"), join(fixtureTemp, "hydra.yaml"));
  cpSync(join(fixtureDir, "type-tests.ts"), join(fixtureTemp, "type-tests.ts"));

  const generatorArgs = [
    "run",
    "--manifest-path",
    join(repoRoot, "Cargo.toml"),
    "-p",
    "hydra-codegen",
    "--bin",
    "hydra-codegen",
    "--",
    "write",
  ];
  run(cargo, generatorArgs);
  const artifactNames = ["cli.rs", "http.rs", "mcp.json", join("ts-client", "index.ts")];
  const first = artifactNames.map((name) => readFileSync(join(generatedDir, name)));

  run(cargo, generatorArgs);
  const second = artifactNames.map((name) => readFileSync(join(generatedDir, name)));
  for (let index = 0; index < artifactNames.length; index += 1) {
    if (!first[index].equals(second[index])) {
      throw new Error(`two generated writes differ for ${artifactNames[index]}`);
    }
  }

  run(cargo, [
    "run",
    "--manifest-path",
    join(repoRoot, "Cargo.toml"),
    "-p",
    "hydra-codegen",
    "--bin",
    "hydra-codegen",
    "--",
    "check",
  ]);
  run(process.execPath, [
    tsc,
    "--target",
    "ES2022",
    "--module",
    "NodeNext",
    "--moduleResolution",
    "NodeNext",
    "--strict",
    "--skipLibCheck",
    "--lib",
    "ES2022,DOM",
    "--noEmit",
    "type-tests.ts",
  ]);
} finally {
  rmSync(tempRoot, { recursive: true, force: true });
}
