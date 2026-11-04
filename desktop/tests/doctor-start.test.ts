/**
 * A built-in doctor chat must greet with starter cards matched to what the
 * doctor can actually do (read agent configs, locate a problem, walk through
 * settings) instead of the generic onboarding trio, whose "ask the doctor"
 * card is meaningless inside a doctor chat.
 * Run from desktop/: node --import tsx tests/doctor-start.test.ts
 */

import assert from "node:assert/strict";

// doctor-start imports ../i18n, which reads localStorage/navigator at import time.
const storage = new Map<string, string>();
Object.defineProperty(globalThis, "localStorage", {
  configurable: true,
  writable: true,
  value: {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => {
      storage.set(key, value);
    },
    removeItem: (key: string) => {
      storage.delete(key);
    },
  },
});
Object.defineProperty(globalThis, "navigator", {
  configurable: true,
  writable: true,
  value: { language: "en" },
});

const { doctorStartCards } = await import("../src/chat/doctor-start");
const { en } = await import("../src/i18n/en");
const { zh } = await import("../src/i18n/zh");

const cards = doctorStartCards();
assert.equal(cards.length, 3, "three starter cards");

for (const card of cards) {
  assert.ok(card.title.trim(), "card has a title");
  assert.ok(card.hint.trim(), "card has a hint");
  assert.ok(card.prompt.trim(), "card drafts a prompt");
}
assert.match(cards[0]!.prompt, /checkup/i, "first card is the checkup");
assert.ok(
  cards[1]!.prompt.trim().endsWith(":"),
  "troubleshoot card invites the person to describe the problem",
);
assert.match(cards[2]!.prompt, /config/i, "third card is the config walkthrough");

// i18n-pairs checks key parity across locales; this checks the cards point at
// real entries in both, so a switched locale never renders "chat.doctorStart…".
const keys = [
  "chat.doctorStartCheckup",
  "chat.doctorStartCheckupHint",
  "chat.doctorStartCheckupPrompt",
  "chat.doctorStartDiagnose",
  "chat.doctorStartDiagnoseHint",
  "chat.doctorStartDiagnosePrompt",
  "chat.doctorStartExplain",
  "chat.doctorStartExplainHint",
  "chat.doctorStartExplainPrompt",
];
for (const [dict, label] of [
  [en, "en"],
  [zh, "zh"],
] as const) {
  for (const key of keys) {
    assert.ok((dict as Record<string, string>)[key]?.trim(), `${label} has copy for ${key}`);
  }
}

console.log("doctor start cards ok");
