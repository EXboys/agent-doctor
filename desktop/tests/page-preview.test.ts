/**
 * The preview page can be told to click, type, and look for words.
 *
 * Run from desktop/:
 *   node --import tsx tests/page-preview.test.ts
 */

import { pageAddress, wrapHtmlForPreview } from "../src/chat/page-preview";

let failures = 0;
function check(name: string, ok: boolean, detail?: unknown): void {
  if (ok) console.log(`ok - ${name}`);
  else {
    failures += 1;
    console.log(`not ok - ${name}`, detail ?? "");
  }
}

const wrapped = wrapHtmlForPreview("<html><head></head><body><button>保存</button></body></html>");
check("the page can receive a click step", wrapped.includes("agent-doctor-ui"));
check("the runner sits in the head", wrapped.indexOf("agent-doctor-ui") < wrapped.indexOf("<body"));

check("a local page address is completed", pageAddress("localhost:3000") === "http://localhost:3000");
check("a full address is kept", pageAddress("https://example.com/app") === "https://example.com/app");
check("plain words are not an address", pageAddress("保存按钮") === null);

if (failures) {
  console.log(`${failures} failed`);
  process.exit(1);
}
