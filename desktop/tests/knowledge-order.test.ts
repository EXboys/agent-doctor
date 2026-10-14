/**
 * Run from desktop/: node --import tsx tests/knowledge-order.test.ts
 */

function assert(cond: boolean, msg: string) {
  if (!cond) throw new Error(msg);
}

const { reorderPagePaths } = await import("../src/chat/knowledge-order.ts");

const paths = ["guides/a.md", "guides/b.md", "guides/c.md"];
assert(
  reorderPagePaths(paths, "guides/c.md", "guides/a.md", "before").join(",") ===
    "guides/c.md,guides/a.md,guides/b.md",
  "moves a page before another page",
);
assert(
  reorderPagePaths(paths, "guides/a.md", "guides/c.md", "after").join(",") ===
    "guides/b.md,guides/c.md,guides/a.md",
  "moves a page after another page",
);
assert(
  reorderPagePaths(paths, "guides/a.md", "topics/a.md", "before") === paths,
  "ignores a target outside the visible category",
);

console.log("knowledge-order ok");
