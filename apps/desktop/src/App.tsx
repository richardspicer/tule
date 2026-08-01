import { useEffect, useState } from "react";
import "./App.css";
import { getApplicationInfo, type ApplicationInfo } from "./platform/application";
import {
  applyThemePreference,
  getNextThemePreference,
  loadThemePreference,
  type ThemePreference,
} from "./theme";

type ConnectionState = "checking" | "connected" | "unavailable";

function App() {
  const [applicationInfo, setApplicationInfo] = useState<ApplicationInfo | null>(null);
  const [connection, setConnection] = useState<ConnectionState>("checking");
  const [theme, setTheme] = useState<ThemePreference>(loadThemePreference);

  useEffect(() => {
    let active = true;

    getApplicationInfo()
      .then((info) => {
        if (active) {
          setApplicationInfo(info);
          setConnection("connected");
        }
      })
      .catch(() => {
        if (active) {
          setConnection("unavailable");
        }
      });

    return () => {
      active = false;
    };
  }, []);

  useEffect(() => {
    applyThemePreference(theme);
  }, [theme]);

  function cycleTheme() {
    setTheme(getNextThemePreference(theme));
  }

  return (
    <main className="app-shell">
      <nav className="topbar" aria-label="Application">
        <div className="wordmark">
          <span className="wordmark-mark" aria-hidden="true">
            t
          </span>
          <span>Tule</span>
        </div>
        <button className="theme-control" type="button" onClick={cycleTheme}>
          <span aria-hidden="true">
            {theme === "dark" ? "Moon" : theme === "light" ? "Sun" : "Auto"}
          </span>
          <span className="sr-only">Appearance: {theme}. Change appearance.</span>
        </button>
      </nav>

      <section className="hero" aria-labelledby="page-title">
        <p className="eyebrow">FOUNDATION 01</p>
        <h1 id="page-title">Make room for the work that matters now.</h1>
        <p className="lede">
          Tule is taking shape as a calm, local workspace for thinking, deciding, and building.
        </p>
      </section>

      <section className="foundation-card" aria-labelledby="foundation-title">
        <div className="card-heading">
          <div>
            <p className="section-label">DESKTOP FOUNDATION</p>
            <h2 id="foundation-title">The first connection</h2>
          </div>
          <span className={`status-pill status-${connection}`} aria-live="polite">
            <span className="status-dot" aria-hidden="true" />
            {connection === "connected"
              ? "Core connected"
              : connection === "checking"
                ? "Checking core"
                : "Desktop required"}
          </span>
        </div>

        <p className="card-copy">
          This build establishes the native desktop shell, its browser-quality interface, and an
          isolated Rust core. Product workflows come next.
        </p>

        <dl className="foundation-details">
          <div>
            <dt>Application</dt>
            <dd>{applicationInfo?.name ?? "Tule"}</dd>
          </div>
          <div>
            <dt>Build</dt>
            <dd>{applicationInfo?.version ?? "0.1.0"}</dd>
          </div>
          <div>
            <dt>Appearance</dt>
            <dd>{theme[0].toUpperCase() + theme.slice(1)}</dd>
          </div>
        </dl>
      </section>

      <footer>
        <span>Local first.</span>
        <span aria-hidden="true">&#183;</span>
        <span>Early foundation.</span>
      </footer>
    </main>
  );
}

export default App;
