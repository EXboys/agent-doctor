/**
 * Open-file tabs and the folder shown beside a file.
 *
 * Run from desktop/: node --import tsx tests/files-open-tabs.test.ts
 */
import { closeOpenTab, parentFolder, rememberOpenTab, tabKey, type OpenFileTab } from "../src/chat/files-open-tabs.ts";

function assert(cond: boolean, msg: string) {
  if (!cond) throw new Error(msg);
  console.log(`ok - ${msg}`);
}

assert(parentFolder("filetest-测试.txt") === "", "a file at the folder root has no parent");
assert(parentFolder("desktop/tests/reply-changes.test.ts") === "desktop/tests", "the parent is the file's folder");

const first: OpenFileTab = {
  key: tabKey("/proj", "src/a.ts"),
  name: "a.ts",
  relative: "src/a.ts",
  root: "/proj",
  external: false,
};
const second: OpenFileTab = {
  key: tabKey("/Users/airlu/Desktop", "filetest-测试.txt"),
  name: "filetest-测试.txt",
  relative: "filetest-测试.txt",
  root: "/Users/airlu/Desktop",
  external: true,
};

const opened = rememberOpenTab(rememberOpenTab([], first), second);
assert(opened.length === 2 && opened[1]?.name === "filetest-测试.txt", "each opened file is a tab");
const again = rememberOpenTab(opened, first);
assert(again.length === 2 && again[1]?.key === first.key, "opening a file again keeps a single tab");

const closed = closeOpenTab(again, first.key);
assert(closed.tabs.length === 1 && closed.next?.key === second.key, "closing the current file selects the neighbor");
const kept = closeOpenTab(again, second.key);
assert(kept.next?.key === first.key && kept.tabs.some((tab) => tab.key === first.key), "closing another file leaves the rest");
assert(closeOpenTab([first], first.key).next === null, "closing the last file clears the selection");
