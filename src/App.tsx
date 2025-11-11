import { useRef, type ReactNode } from "react";
import { BrowserRouter as Router } from "react-router-dom";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Input } from "@/components/ui/input";
import ConnectionsPage, { type ConnectionsPageHandle } from "@/pages/connections";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import type { LucideIcon } from "lucide-react";
import { Database, Plus, Search } from "lucide-react";
import "./App.css";

type NavItem = {
  value: "connections";
  label: string;
  description: string;
  icon: LucideIcon;
};

const NAV_ITEMS: NavItem[] = [
  {
    value: "connections",
    label: "Connections",
    description: "Create connection…",
    icon: Database,
  },
];

const Panel = ({ children }: { children: ReactNode }) => <section className="panel">{children}</section>;

function App() {
  const connectionsRef = useRef<ConnectionsPageHandle | null>(null);

  return (
    <Router>
      <Tabs defaultValue="connections" className="app-shell">
        <div className="app-layout">
          <aside className="nav-rail">
            <TabsList className="nav-rail__tabs" aria-label="Workspace areas">
              {NAV_ITEMS.map(({ value, label, description, icon: Icon }) => (
                <TabsTrigger key={value} value={value} className="nav-rail__trigger">
                  <Icon className="nav-rail__trigger-icon" size={18} />
                  <div className="nav-rail__trigger-text">
                    <span>{label}</span>
                    <small>{description}</small>
                  </div>
                </TabsTrigger>
              ))}
            </TabsList>
          </aside>

          <main className="main-stage">
            <header className="main-stage__toolbar">
              <div className="toolbar__left">
                <DropdownMenu>
                  <DropdownMenuTrigger asChild>
                    <button className="toolbar__icon-button" type="button" aria-label="Add database connection">
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
                  <Input placeholder="Search for connection… (Ctrl F)" className="toolbar__search-input" />
                </div>
              </div>
            </header>

            <section className="main-stage__content">
              <TabsContent value="connections" className="workspace__panel" forceMount>
                <Panel>
                  <ConnectionsPage ref={connectionsRef} />
                </Panel>
              </TabsContent>
            </section>
          </main>
        </div>
      </Tabs>
    </Router>
  );
}

export default App;
