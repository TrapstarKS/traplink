const letters = "ABCDEFGHIJKLMNOPQRSTUVWXYZ".split("").map(letter => [`Key${letter}`, letter] as const);
const digits = "0123456789".split("").map(digit => [`Digit${digit}`, digit] as const);
const functions = Array.from({ length: 24 }, (_, index) => [`F${index + 1}`, `F${index + 1}`] as const);
const named: Array<readonly [string, string]> = [
  ["Escape", "Esc"], ["Minus", "-"], ["Equal", "="], ["Backspace", "Backspace"], ["Tab", "Tab"],
  ["BracketLeft", "["], ["BracketRight", "]"], ["Enter", "Enter"], ["Semicolon", ";"], ["Quote", "'"],
  ["Backquote", "`"], ["Backslash", "\\"], ["Comma", ","], ["Period", "."], ["Slash", "/"], ["IntlBackslash", "\\ (ABNT)"],
  ["IntlRo", "/ (ABNT)"], ["Space", "Space"], ["CapsLock", "Caps Lock"], ["ScrollLock", "Scroll Lock"], ["NumLock", "Num Lock"],
  ["PrintScreen", "Print Screen"], ["ContextMenu", "Menu"],
  ["ControlLeft", "Left Ctrl"], ["ControlRight", "Right Ctrl"], ["ShiftLeft", "Left Shift"], ["ShiftRight", "Right Shift"],
  ["AltLeft", "Alt"], ["AltRight", "AltGr"], ["MetaLeft", "Windows"], ["MetaRight", "Right Windows"],
  ["Insert", "Insert"], ["Delete", "Delete"], ["Home", "Home"], ["End", "End"], ["PageUp", "Page Up"], ["PageDown", "Page Down"],
  ["ArrowUp", "Up Arrow"], ["ArrowDown", "Down Arrow"], ["ArrowLeft", "Left Arrow"], ["ArrowRight", "Right Arrow"],
  ["Numpad0", "Num 0"], ["Numpad1", "Num 1"], ["Numpad2", "Num 2"], ["Numpad3", "Num 3"], ["Numpad4", "Num 4"],
  ["Numpad5", "Num 5"], ["Numpad6", "Num 6"], ["Numpad7", "Num 7"], ["Numpad8", "Num 8"], ["Numpad9", "Num 9"],
  ["NumpadAdd", "Num +"], ["NumpadSubtract", "Num -"], ["NumpadMultiply", "Num *"], ["NumpadDivide", "Num /"],
  ["NumpadDecimal", "Num ,"], ["NumpadComma", "Num . (ABNT)"], ["NumpadEnter", "Num Enter"],
  ["AudioVolumeMute", "Mute"], ["AudioVolumeDown", "Volume down"], ["AudioVolumeUp", "Volume up"],
  ["MediaPlayPause", "Play/Pause"], ["MediaStop", "Stop media"], ["MediaTrackNext", "Next track"], ["MediaTrackPrevious", "Previous track"],
];
export const mouseSources = { MouseMiddle: "Middle button", MouseBack: "Back button (X1)", MouseForward: "Forward button (X2)" } as const;
const keyLabels = new Map<string, string>([...letters, ...digits, ...functions, ...named]);

export type Modifier = "Ctrl" | "Shift" | "Alt" | "Win";
export const modifiers: Modifier[] = ["Ctrl", "Shift", "Alt", "Win"];
const modifierCodes = new Set(["ControlLeft", "ControlRight", "ShiftLeft", "ShiftRight", "AltLeft", "AltRight", "MetaLeft", "MetaRight"]);
const disabledWords = new Set(["disabled", "disable", "desativar", "desativado", "none", "nenhum"]);

export function isSupportedKey(code: string): boolean { return keyLabels.has(code); }
export function isModifierKey(code: string): boolean { return modifierCodes.has(code); }

function canonicalKey(token: string): string | undefined {
  const trimmed = token.trim();
  for (const code of keyLabels.keys()) if (code.toLowerCase() === trimmed.toLowerCase()) return code;
  if (/^[a-z]$/i.test(trimmed)) return `Key${trimmed.toUpperCase()}`;
  if (/^[0-9]$/.test(trimmed)) return `Digit${trimmed}`;
  return undefined;
}
function modifierOf(token: string): Modifier | undefined {
  const lower = token.trim().toLowerCase();
  if (lower === "ctrl" || lower === "control") return "Ctrl";
  if (lower === "shift") return "Shift";
  if (lower === "alt") return "Alt";
  if (["win", "windows", "meta", "super", "cmd"].includes(lower)) return "Win";
  return undefined;
}

export function sourceLabel(value: string): string {
  const mouse = Object.entries(mouseSources).find(([code]) => code.toLowerCase() === value.trim().toLowerCase());
  if (mouse) return mouse[1];
  const key = canonicalKey(value);
  return key ? keyLabels.get(key)! : value;
}

export type Target = { kind: "key"; key: string } | { kind: "shortcut"; modifiers: Modifier[]; key: string } | { kind: "disabled" } | { kind: "custom"; raw: string };

export function parseTarget(value: string): Target {
  const trimmed = value.trim();
  if (!trimmed) return { kind: "key", key: "" };
  if (disabledWords.has(trimmed.toLowerCase())) return { kind: "disabled" };
  const found: Modifier[] = [];
  let key = "";
  for (const token of trimmed.split("+")) {
    const modifier = modifierOf(token);
    if (modifier) { if (!found.includes(modifier)) found.push(modifier); continue; }
    const code = canonicalKey(token);
    if (!code || key) return { kind: "custom", raw: trimmed };
    key = code;
  }
  if (!found.length) return { kind: "key", key };
  return { kind: "shortcut", modifiers: modifiers.filter(modifier => found.includes(modifier)), key };
}

export function formatTarget(target: Target): string {
  switch (target.kind) {
    case "disabled": return "disabled";
    case "custom": return target.raw;
    case "key": return target.key;
    case "shortcut": return [...modifiers.filter(modifier => target.modifiers.includes(modifier)), ...(target.key ? [target.key] : [])].join("+");
  }
}

export function targetLabel(value: string): string {
  const target = parseTarget(value);
  switch (target.kind) {
    case "disabled": return "Disabled";
    case "custom": return target.raw;
    case "key": return keyLabels.get(target.key) ?? target.key;
    case "shortcut": return [...target.modifiers, ...(target.key ? [keyLabels.get(target.key) ?? target.key] : [])].join(" + ");
  }
}

export function keyLabel(code: string): string { return keyLabels.get(code) ?? code; }
