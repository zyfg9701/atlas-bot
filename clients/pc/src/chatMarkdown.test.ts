import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";

const dom = new JSDOM("<!DOCTYPE html><html><body></body></html>");
const g = globalThis as unknown as Record<string, unknown>;
g.window = dom.window;
g.document = dom.window.document;
g.Node = dom.window.Node;
g.Element = dom.window.Element;
g.HTMLElement = dom.window.HTMLElement;
g.DocumentFragment = dom.window.DocumentFragment;
g.DOMParser = dom.window.DOMParser;
g.NodeFilter = dom.window.NodeFilter;

const { renderChatMarkdown } = await import("./chatMarkdown.ts");

describe("renderChatMarkdown", () => {
  it("renders bold, lists, and inline code", () => {
    const html = renderChatMarkdown("**粗体** 和 *斜体*\n\n- 一项\n- 两项\n\n`npm test`");
    assert.match(html, /<strong>粗体<\/strong>/);
    assert.match(html, /<em>斜体<\/em>/);
    assert.match(html, /<li>一项<\/li>/);
    assert.match(html, /<li>两项<\/li>/);
    assert.match(html, /<code>npm test<\/code>/);
    assert.equal(html.includes("**"), false);
  });

  it("renders a heading and a safe link", () => {
    const html = renderChatMarkdown("## 标题\n\n[docs](https://example.com/a)");
    assert.match(html, /<h2>标题<\/h2>/);
    assert.match(html, /href="https:\/\/example.com\/a"/);
    assert.match(html, /rel="noopener noreferrer"/);
    assert.match(html, /target="_blank"/);
  });

  it("strips script and javascript urls", () => {
    const html = renderChatMarkdown(
      '<script>alert(1)</script>\n\n[x](javascript:alert(1))\n\n<img src=x onerror=alert(1)>',
    );
    assert.equal(html.toLowerCase().includes("<script"), false);
    assert.equal(html.toLowerCase().includes("javascript:"), false);
    assert.equal(html.toLowerCase().includes("<img"), false);
    assert.equal(html.toLowerCase().includes("onerror"), false);
  });

  it("leaves hub channel names as text", () => {
    const html = renderChatMarkdown("see hub:turn_finished in the debug log");
    assert.match(html, /hub:turn_finished/);
    assert.equal(html.includes("—— live events ——"), false);
  });
});
