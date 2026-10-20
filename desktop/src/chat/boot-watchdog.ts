// Boot watchdog for the Ask window. Kept as its own module so the page can
// ship a strict `script-src 'self'`: it must survive chat.ts failing to boot,
// so it never imports from the chat graph.
window.addEventListener("error", function () {
  const log = document.getElementById("chat-log");
  if (!log || log.querySelector(".chat-session, .chat-bubble.assistant, .chat-bubble.user")) return;
  if (document.getElementById("chat-boot-error")) return;
  const box = document.createElement("div");
  box.id = "chat-boot-error";
  box.className = "chat-bubble meta";
  box.style.margin = "12px";
  box.textContent =
    "对话页脚本出错。请关掉 Ask，从主页重新点进问答；或完全退出 Agent Doctor 再开。";
  log.appendChild(box);
});

// Stuck Ask webviews keep the shell but never boot chat.ts — force a reload once.
window.setTimeout(function () {
  const win = window as Window & { __AD_ASK_BOOTED__?: boolean };
  if (win.__AD_ASK_BOOTED__) return;
  const sessions = document.getElementById("chat-sessions");
  const emptyList = sessions && sessions.childElementCount === 0;
  const placeholder = document.getElementById("chat-boot-placeholder");
  if (!emptyList && !placeholder) return;
  if (sessionStorage.getItem("ad-ask-reload") === "1") {
    const log = document.getElementById("chat-log");
    if (log && !document.getElementById("chat-boot-error")) {
      const box = document.createElement("div");
      box.id = "chat-boot-error";
      box.className = "chat-bubble meta";
      box.style.margin = "12px";
      box.textContent =
        "会话列表加载失败。请完全退出 Agent Doctor，确认开发服务还在跑，再重新打开。";
      log.appendChild(box);
    }
    return;
  }
  sessionStorage.setItem("ad-ask-reload", "1");
  location.reload();
}, 2500);
