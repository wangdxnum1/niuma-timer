#!/usr/bin/env node
// Run every scripts/test_*.js in its own child process and stop on the first
// failure. Each script mutates globals (Date / $), so sharing one process
// would cross-contaminate them - spawnSync isolation is mandatory.
// Exit 0 only when all scripts pass.
"use strict";

const { spawnSync } = require("child_process");
const fs = require("fs");
const path = require("path");

const self = path.basename(__filename);
const tests = fs.readdirSync(__dirname)
  .filter(f => /^test_.+\.js$/.test(f) && f !== self)
  .sort();

if (tests.length === 0) {
  console.error("[run_all] no test_*.js found in scripts/");
  process.exit(1);
}

for (const t of tests) {
  const r = spawnSync(process.execPath, [path.join(__dirname, t)], { stdio: "inherit" });
  if (r.error) {
    console.error(`[run_all] ${t} could not start: ${r.error.message}`);
    process.exit(1);
  }
  if (r.status !== 0) {
    console.error(`[run_all] ${t} FAILED (exit ${r.status})`);
    process.exit(r.status || 1);
  }
}
console.log(`[run_all] ${tests.length} test scripts, all passed`);
