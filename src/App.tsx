import { useEffect, useRef, useState, type ReactNode } from "react";
import { BrowserRouter as Router } from "react-router-dom";
import ConnectionsPage, { type ConnectionsPageHandle } from "@/pages/connections";
import SafeSyncPage from "@/pages/safesync";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Database, Plus, Search, ShieldCheck } from "lucide-react";
import "./App.css";

const Panel = ({ children }: { children: ReactNode }) => <section className="panel">{children}</section>;

function App() {
  const connectionsRef = useRef<ConnectionsPageHandle | null>(null);
  const searchInputRef = useRef<HTMLInputElement | null>(null);
  const [activeView, setActiveView] = useState<"connections" | "safesync">("connections");
  const [searchQuery, setSearchQuery] = useState("");

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "f") {
        if (activeView === "connections" && searchInputRef.current) {
          event.preventDefault();
          searchInputRef.current.focus();
        }
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [activeView]);

  return (
    <Router>
      <div className="app-shell">
        <div className="app-layout">
          <aside className="nav-rail" data-tauri-drag-region>
            <div className="nav-rail__top">
              <div className="nav-rail__brand">
                <img src="/logo_dbpinball.png" alt="DB Pinball logo" className="nav-rail__logo" />
                <div className="nav-rail__text">
                  <h1 className="nav-rail__title">DB Pinball</h1>
                  <p className="nav-rail__subtitle">Version {__APP_VERSION__}</p>
                </div>
              </div>

              <nav className="nav-rail__cta-stack" aria-label="Primary navigation">
                <button
                  type="button"
                  className={`nav-rail__cta${activeView === "connections" ? " nav-rail__cta--active" : ""}`}
                  aria-label="Connections"
                  onClick={() => setActiveView("connections")}
                  data-tauri-drag-region="false"
                >
                  <Database size={15} className="nav-rail__cta-icon" />
                  <span>Connections</span>
                </button>
                <button
                  type="button"
                  className={`nav-rail__cta${activeView === "safesync" ? " nav-rail__cta--active" : ""}`}
                  aria-label="Safe Sync"
                  onClick={() => setActiveView("safesync")}
                  data-tauri-drag-region="false"
                >
                  <ShieldCheck size={15} className="nav-rail__cta-icon" />
                  <span>Safe Sync</span>
                </button>
              </nav>
            </div>

            <div className="nav-rail__footer">
              <button
                type="button"
                className="nav-rail__action nav-rail__action--primary"
                aria-label="Create connection"
                onClick={() => {
                  setActiveView("connections");
                  connectionsRef.current?.openNewConnection();
                }}
                data-tauri-drag-region="false"
              >
                <Plus size={15} />
                <span>New Connection</span>
              </button>
            </div>
          </aside>

          <main className="main-stage">
            <header className="main-stage__toolbar" data-tauri-drag-region>
              {activeView === "connections" ? (
                <div className="toolbar__left">
                  <DropdownMenu>
                    <DropdownMenuTrigger asChild>
                      <button
                        className="toolbar__icon-button"
                        type="button"
                        aria-label="Add database connection"
                        data-tauri-drag-region="false"
                      >
                        <Plus size={16} />
                      </button>
                    </DropdownMenuTrigger>
                    <DropdownMenuContent side="bottom" align="start" sideOffset={8} className="toolbar__menu">
                      <DropdownMenuItem
                        className="toolbar__menu-item"
                        onSelect={() => connectionsRef.current?.openNewConnection("mysql")}
                      >
                        Add MySQL Connection
                      </DropdownMenuItem>
                      <DropdownMenuItem
                        className="toolbar__menu-item"
                        onSelect={() => connectionsRef.current?.openNewConnection("postgres")}
                      >
                        Add PostgreSQL Connection
                      </DropdownMenuItem>
                    </DropdownMenuContent>
                  </DropdownMenu>
                  <div className="toolbar__search">
                    <Search className="toolbar__search-icon" size={15} />
                    <input
                      ref={searchInputRef}
                      type="text"
                      value={searchQuery}
                      onChange={(e) => setSearchQuery(e.target.value)}
                      placeholder="Filter connections… (Ctrl+F)"
                      className="toolbar__search-input"
                      data-tauri-drag-region="false"
                    />
                  </div>
                </div>
              ) : (
                <div className="toolbar__left">
                  <span className="toolbar__section-title">Safe Sync</span>
                </div>
              )}
            </header>

            <section className="main-stage__content">
              <Panel>
                {activeView === "safesync" ? (
                  <SafeSyncPage />
                ) : (
                  <ConnectionsPage ref={connectionsRef} searchQuery={searchQuery} />
                )}
              </Panel>
            </section>
          </main>
        </div>
      </div>
    </Router>
  );
}

export default App;
