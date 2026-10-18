import test from "node:test";
import assert from "node:assert/strict";
import { polish, t, setLanguage, language } from "../i18n.js";
test("Polish messages retain all parameters and switch back to English", () => {
  for (const [key, value] of Object.entries(polish)) {
    assert.ok(value.trim());
    assert.deepEqual([...key.matchAll(/\{(\w+)\}/g)].map(m => m[1]).sort(), [...value.matchAll(/\{(\w+)\}/g)].map(m => m[1]).sort(), key);
  }
  setLanguage("pl"); assert.equal(language(), "pl");
  assert.equal(t("Edit {line} at {stop}", { line: "4", stop: "Most Teatralny" }), "Edytuj linię 4, przystanek Most Teatralny");
  assert.equal(t("Firmware verified"), "Aplikacja zweryfikowana");
  setLanguage("en"); assert.equal(t("Firmware verified"), "Firmware verified");
  setLanguage("xx"); assert.equal(language(), "en");
});
