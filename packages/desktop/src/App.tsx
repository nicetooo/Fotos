import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { 
  Image as ImageIcon, 
  Search, 
  Heart, 
  Trash2, 
  Album, 
  Settings,
  Share2,
  Clock,
  LayoutGrid
} from "lucide-react";
import "./App.css";

function App() {
  const [coreVersion, setCoreVersion] = useState<string>("Loading...");
  const [activeTab, setActiveTab] = useState("all");

  useEffect(() => {
    async function loadVersion() {
      try {
        const version = await invoke("get_core_version");
        setCoreVersion(version as string);
      } catch (err) {
        console.error("Failed to load core version:", err);
        setCoreVersion("Unknown");
      }
    }
    loadVersion();
  }, []);

  const navItems = [
    { id: "all", icon: <ImageIcon size={20} />, label: "All Photos" },
    { id: "albums", icon: <Album size={20} />, label: "Albums" },
    { id: "recent", icon: <Clock size={20} />, label: "Recent" },
    { id: "favorites", icon: <Heart size={20} />, label: "Favorites" },
    { id: "trash", icon: <Trash2 size={20} />, label: "Trash" },
  ];

  return (
    <div className="app-container">
      <aside className="sidebar">
        <div className="sidebar-header">
          <div className="logo-text">Fotos</div>
        </div>
        
        <nav>
          {navItems.map((item) => (
            <div 
              key={item.id}
              className={`nav-item ${activeTab === item.id ? 'active' : ''}`}
              onClick={() => setActiveTab(item.id)}
            >
              {item.icon}
              <span>{item.label}</span>
            </div>
          ))}
        </nav>

        <div style={{ marginTop: 'auto' }}>
          <div className="nav-item">
            <Settings size={20} />
            <span>Settings</span>
          </div>
        </div>
      </aside>

      <main className="main-content">
        <header className="top-bar">
          <div className="search-container">
            <Search className="search-icon" size={18} />
            <input 
              type="text" 
              className="search-input" 
              placeholder="Search your memories..."
            />
          </div>
          
          <div style={{ display: 'flex', alignItems: 'center', gap: '16px' }}>
            <div className="version-badge">
              Core v{coreVersion}
            </div>
            <button className="nav-item" style={{ background: 'none', border: 'none', cursor: 'pointer' }}>
               <Share2 size={20} />
            </button>
          </div>
        </header>

        <div className="photo-grid">
          {[...Array(12)].map((_, i) => (
            <div key={i} className="photo-card">
              <div className="photo-placeholder">
                <ImageIcon size={48} strokeWidth={1} style={{ opacity: 0.2 }} />
              </div>
            </div>
          ))}
        </div>
      </main>
    </div>
  );
}

export default App;
