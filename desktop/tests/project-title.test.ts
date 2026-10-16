/**
 * Run from desktop/: node --import tsx tests/project-title.test.ts
 */

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

function assert(cond: boolean, msg: string) {
  if (!cond) throw new Error(msg);
}

async function main() {
  const { listProjectGroupsForSidebar, projectDisplayLabel } = await import(
    "../src/chat/session-workspace"
  );
  const doc = {
    active: "LEYAN_PROJECT_OLD",
    workspaces: {
      LEYAN_PROJECT_OLD: { path: "/tmp/leyan", title: "乐言" },
      default: { path: "/tmp/home" },
    },
  };
  assert(projectDisplayLabel("LEYAN_PROJECT_OLD", doc) === "乐言", "custom title");
  assert(projectDisplayLabel("default", doc) === "default", "folder name when untitled");
  assert(projectDisplayLabel("missing", doc) === "missing", "unknown project keeps its key");
  const groups = listProjectGroupsForSidebar([], doc, "其他");
  const leyan = groups.find((group) => group.name === "LEYAN_PROJECT_OLD");
  assert(leyan?.label === "乐言", "sidebar uses the custom title");
  assert(groups.find((group) => group.name === "default")?.label === "default", "untitled row");
  console.log("project title ok");
}

void main();
