/** Push the phone's view of each computer chat. Bursts of turn updates share one send. */
let push: (() => void) | null = null;
let timer = 0;

export function bindPhoneChats(publish: () => void): void {
  push = publish;
}

export function publishPhoneChats(): void {
  if (!push) return;
  window.clearTimeout(timer);
  timer = window.setTimeout(() => push?.(), 40);
}
