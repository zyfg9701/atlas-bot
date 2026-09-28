/**
 * Chat bubble Markdown. `marked` produces HTML; DOMPurify sanitizes it
 * before any `innerHTML`. Debug panes do not use this helper.
 */
import DOMPurify from "dompurify";
import { Marked } from "marked";

const marked = new Marked({
  async: false,
  gfm: true,
  breaks: true,
});

type Purifier = {
  sanitize: (dirty: string, cfg?: object) => string;
  addHook?: (hook: string, cb: (node: Element) => void) => void;
};

const SANITIZE = {
  USE_PROFILES: { html: true },
  FORBID_TAGS: [
    "img",
    "video",
    "audio",
    "iframe",
    "object",
    "embed",
    "form",
    "input",
    "button",
    "style",
    "svg",
    "math",
  ],
  FORBID_ATTR: ["style"],
};

let purifier: Purifier | null = null;

function getPurifier(): Purifier {
  if (purifier) return purifier;
  const exported = DOMPurify as unknown as Purifier & ((root: unknown) => Purifier);
  let instance: Purifier;
  if (typeof exported.sanitize === "function") {
    instance = exported;
  } else {
    const w = (globalThis as { window?: unknown }).window;
    if (!w) throw new Error("DOMPurify requires a window");
    instance = exported(w);
  }
  instance.addHook?.("afterSanitizeAttributes", (node) => {
    if (node.tagName === "A") {
      node.setAttribute("rel", "noopener noreferrer");
      node.setAttribute("target", "_blank");
    }
  });
  purifier = instance;
  return instance;
}

function escapeHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

/** Render one message body to sanitized HTML. */
export function renderChatMarkdown(src: string): string {
  const source = src ?? "";
  let html: string;
  try {
    const parsed = marked.parse(source, { async: false });
    html = typeof parsed === "string" ? parsed : escapeHtml(source);
  } catch {
    html = escapeHtml(source);
  }
  return getPurifier().sanitize(html, SANITIZE);
}
