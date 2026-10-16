/**
 * Run from desktop/: node --import tsx tests/hands-intent.test.ts
 */

function assert(cond: boolean, msg: string) {
  if (!cond) throw new Error(msg);
}

globalThis.localStorage = {
  getItem: () => "zh",
  setItem: () => undefined,
  removeItem: () => undefined,
  clear: () => undefined,
  key: () => null,
  length: 0,
};

const { handsBlockForText, promptRequestsComputerUse } = await import("../src/chat/hands-intent.ts");

assert(handsBlockForText("帮我打开浏览器看看订单", false, false) === "browser", "browser stays off");
assert(handsBlockForText("帮我打开浏览器看看订单", true, false) === null, "browser switch allows it");
assert(promptRequestsComputerUse("去千牛里回这句"), "qianniu needs the computer");
assert(handsBlockForText("去千牛里回这句", false, false) === "computer", "computer stays off");
assert(handsBlockForText("去千牛里回这句", false, true) === null, "computer switch allows it");
assert(
  handsBlockForText("打开浏览器，再去千牛发货", false, false) === "both",
  "both switches",
);
assert(handsBlockForText("这件什么时候发货", false, false) === null, "plain chat is free");
console.log("hands-intent ok");
