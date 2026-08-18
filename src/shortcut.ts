const codeAliases: Record<string, string> = {
  Space: "Space",
  Escape: "Escape",
  Enter: "Enter",
  Tab: "Tab",
  Backspace: "Backspace",
  Delete: "Delete",
  Insert: "Insert",
  Home: "Home",
  End: "End",
  PageUp: "PageUp",
  PageDown: "PageDown",
  ArrowUp: "ArrowUp",
  ArrowDown: "ArrowDown",
  ArrowLeft: "ArrowLeft",
  ArrowRight: "ArrowRight",
  Backquote: "Backquote",
  Backslash: "Backslash",
  BracketLeft: "BracketLeft",
  BracketRight: "BracketRight",
  Comma: "Comma",
  Equal: "Equal",
  Minus: "Minus",
  Period: "Period",
  Quote: "Quote",
  Semicolon: "Semicolon",
  Slash: "Slash",
};

function mainKey(event: KeyboardEvent): string | null {
  if (/^Key[A-Z]$/.test(event.code)) return event.code.slice(3);
  if (/^Digit[0-9]$/.test(event.code)) return event.code.slice(5);
  if (/^F([1-9]|1[0-2])$/.test(event.code)) return event.code;
  if (/^Numpad[0-9]$/.test(event.code)) return event.code;
  return codeAliases[event.code] ?? null;
}

export function shortcutFromKeyboardEvent(event: KeyboardEvent): string | null {
  const key = mainKey(event);
  if (!key) return null;

  const parts: string[] = [];
  if (event.ctrlKey) parts.push("Ctrl");
  if (event.altKey) parts.push("Alt");
  if (event.shiftKey) parts.push("Shift");
  if (event.metaKey) parts.push("Super");
  parts.push(key);
  return parts.join("+");
}

export function bindShortcutCapture(
  button: HTMLButtonElement,
  initialValue: string,
  onChange: (shortcut: string) => void,
): void {
  let value = initialValue;
  const label = button.querySelector<HTMLElement>("[data-hotkey-value]");

  const display = (text: string): void => {
    if (label) label.textContent = text || "press a shortcut";
  };

  display(value);
  button.addEventListener("focus", () => {
    button.classList.add("is-recording");
    display("listening…");
  });
  button.addEventListener("blur", () => {
    button.classList.remove("is-recording");
    display(value);
  });
  button.addEventListener("keydown", (event) => {
    event.preventDefault();
    event.stopPropagation();
    if (event.code === "Escape") {
      button.blur();
      return;
    }
    const shortcut = shortcutFromKeyboardEvent(event);
    if (!shortcut) {
      display("add a main key…");
      return;
    }
    value = shortcut;
    display(value);
    onChange(value);
    button.blur();
  });
}
