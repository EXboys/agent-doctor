/**
 * Run from desktop/: node --import tsx tests/install-failure.test.ts
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
Object.defineProperty(globalThis, "navigator", {
  configurable: true,
  writable: true,
  value: { language: "zh" },
});

const { installFailureSentenceFor } = await import("../src/friendly-error.ts");
const { t } = await import("../src/i18n.ts");

const manifest =
  "Fatal: Could not connect to the release server to download the manifest. Please check your internet connection or firewall settings.";
if (installFailureSentenceFor("antigravity", manifest) !== t("runtime.installFailNetwork")) {
  throw new Error(`network install should stay a network sentence, got ${installFailureSentenceFor("antigravity", manifest)}`);
}

const desktop = "Antigravity 桌面软件已在本机，命令行还没装。";
if (installFailureSentenceFor("antigravity", desktop) !== t("runtime.installFailMissingCli")) {
  throw new Error(`desktop-only install should say the app is already here, got ${installFailureSentenceFor("antigravity", desktop)}`);
}

const desktopOffline = `${desktop}\n${manifest}`;
if (installFailureSentenceFor("antigravity", desktopOffline) !== t("runtime.installFailMissingCliNetwork")) {
  throw new Error(`desktop plus a blocked download should say both, got ${installFailureSentenceFor("antigravity", desktopOffline)}`);
}

const other = "install script finished but binary still not found on PATH";
if (installFailureSentenceFor("antigravity", other) !== t("runtime.installFailedExperimental")) {
  throw new Error(`other experimental failures keep the short sentence, got ${installFailureSentenceFor("antigravity", other)}`);
}
