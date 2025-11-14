import { useRef, type ReactNode } from "react";
import { BrowserRouter as Router } from "react-router-dom";
import ConnectionsPage, { type ConnectionsPageHandle } from "@/pages/connections";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Plus, Search, UploadCloud, RotateCcw, PlugZap } from "lucide-react";
import "./App.css";

const Panel = ({ children }: { children: ReactNode }) => <section className="panel">{children}</section>;

function App() {
  const connectionsRef = useRef<ConnectionsPageHandle | null>(null);

  return (
    <Router>
      <div className="app-shell">
        <div className="app-layout">
          <aside className="nav-rail" data-tauri-drag-region>
            <div className="nav-rail__brand">
              <img src="/logo_dbpinball.png" alt="DB Pinball logo" className="nav-rail__logo" />
              <div className="nav-rail__text">
                <h1 className="nav-rail__title">DB Pinball</h1>
                <p className="nav-rail__subtitle">Version 0.1.4 (preview)</p>
                <p className="nav-rail__notice">You are using the free trial</p>
              </div>
            </div>

            <div className="nav-rail__actions">
              <button type="button" className="nav-rail__action nav-rail__action--primary" data-tauri-drag-region="false">
                Purchase
              </button>
              <button type="button" className="nav-rail__action" data-tauri-drag-region="false">
                Activate
              </button>
            </div>

            <div className="nav-rail__cta-stack">
              <button
                type="button"
                className="nav-rail__cta"
                aria-label="Backup database"
                data-tauri-drag-region="false"
              >
                <UploadCloud size={16} className="nav-rail__cta-icon" />
                <span>Backup database…</span>
              </button>
              <button
                type="button"
                className="nav-rail__cta"
                aria-label="Restore databases"
                data-tauri-drag-region="false"
              >
                <RotateCcw size={16} className="nav-rail__cta-icon" />
                <span>Restore databases…</span>
              </button>
              <button
                type="button"
                className="nav-rail__cta"
                aria-label="Create connection"
                onClick={() => connectionsRef.current?.openNewConnection()}
                data-tauri-drag-region="false"
              >
                <PlugZap size={16} className="nav-rail__cta-icon" />
                <span>Create connection…</span>
              </button>
            </div>
          </aside>

          <main className="main-stage">
            <header className="main-stage__toolbar" data-tauri-drag-region>
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
                  <Search className="toolbar__search-icon" size={16} />
                  <input
                    type="text"
                    placeholder="Search for connection… (Ctrl F)"
                    className="toolbar__search-input"
                    data-tauri-drag-region="false"
                  />
                </div>
              </div>
            </header>

            <section className="main-stage__content">
              <Panel>
                <ConnectionsPage ref={connectionsRef} />
              </Panel>
            </section>
          </main>
        </div>
      </div>
    </Router>
  );
}

export default App;
