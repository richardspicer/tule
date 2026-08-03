import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterEach, vi } from "vitest";

Object.defineProperty(window, "matchMedia", {
  writable: true,
  value: vi.fn().mockImplementation((query: string) => ({
    matches: false,
    media: query,
    onchange: null,
    addListener: vi.fn(),
    removeListener: vi.fn(),
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
    dispatchEvent: vi.fn(),
  })),
});

Element.prototype.scrollIntoView = vi.fn();

if (typeof document.queryCommandEnabled !== "function") {
  document.queryCommandEnabled = () => false;
}

if (typeof document.execCommand !== "function") {
  document.execCommand = () => false;
}

afterEach(() => {
  cleanup();
  delete document.documentElement.dataset.theme;
});
