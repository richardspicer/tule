import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterEach, beforeAll, vi } from "vitest";

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

function isUnexpectedReactConsoleTraffic(args: unknown[]): boolean {
  const message = args
    .map((arg) => {
      if (typeof arg === "string") {
        return arg;
      }
      if (arg instanceof Error) {
        return arg.message;
      }
      return "";
    })
    .join(" ");

  return (
    message.includes("Warning:") ||
    message.includes("Error:") ||
    message.includes("React") ||
    message.includes("Invalid hook call") ||
    message.includes("Cannot update a component")
  );
}

beforeAll(() => {
  const originalError = console.error.bind(console);
  const originalWarn = console.warn.bind(console);

  vi.spyOn(console, "error").mockImplementation((...args: unknown[]) => {
    if (isUnexpectedReactConsoleTraffic(args)) {
      throw new Error(
        `Unexpected React console.error in tests:\n${args
          .map((arg) => (typeof arg === "string" ? arg : String(arg)))
          .join(" ")}`,
      );
    }
    originalError(...args);
  });

  vi.spyOn(console, "warn").mockImplementation((...args: unknown[]) => {
    if (isUnexpectedReactConsoleTraffic(args)) {
      throw new Error(
        `Unexpected React console.warn in tests:\n${args
          .map((arg) => (typeof arg === "string" ? arg : String(arg)))
          .join(" ")}`,
      );
    }
    originalWarn(...args);
  });
});

afterEach(() => {
  cleanup();
  delete document.documentElement.dataset.theme;
});
