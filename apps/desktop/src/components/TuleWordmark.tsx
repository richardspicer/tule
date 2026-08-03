const tuleWordmark = [
  "▀▀▀▀█▀▀▀ ██    ██ ██      ██▀▀▀▀▀▀",
  "   ██    ██    ██ ██      ██▄▄▄▄▄ ",
  "   ██    ██    ██ ██      ██      ",
  "   ██    ██▄▄▄▄▄█ ██▄▄▄▄▄ ██▄▄▄▄▄▄",
].join("\n");

export function TuleWordmark() {
  return (
    <div className="wordmark session-wordmark" role="img" aria-label="TULE">
      <pre className="wordmark-art" aria-hidden="true">
        {tuleWordmark}
      </pre>
    </div>
  );
}
