import test from "node:test";
import assert from "node:assert/strict";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { Reader, openReader } from "../index.mjs";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const vcfPath = path.join(__dirname, "..", "..", "..", "tests", "t.vcf.gz");

test("Header.get includes Description and undefined for missing", () => {
  const reader = new Reader(vcfPath);

  const dp = reader.header.get("INFO", "DP");
  assert.ok(dp);
  assert.equal(dp.id, "DP");
  assert.equal(dp.type, "Integer");
  assert.equal(dp.number, "1");
  assert.equal(dp.description, "Depth");

  const nope = reader.header.get("INFO", "NOPE");
  assert.equal(nope, undefined);

  reader.close();
});

test("openReader returns usable reader", async () => {
  const reader = await openReader(vcfPath);
  assert.equal(reader.hasIndex(), true);

  const dp = reader.header.get("INFO", "DP");
  assert.ok(dp);
  assert.equal(dp.description, "Depth");

  reader.close();
});
