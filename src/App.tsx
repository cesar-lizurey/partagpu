import { useState, useEffect, useRef } from "react";
import { MySharing } from "./pages/MySharing";
import { MyUsage } from "./pages/MyUsage";
import { Guide } from "./pages/Guide";
import { RoomSetup } from "./components/RoomSetup";
import { getMachineInfo, setDisplayName } from "./lib/api";
import type { MachineInfo } from "./lib/api";
import "./styles.css";

type Tab = "sharing" | "usage" | "guide";

const TABS: { id: Tab; label: string }[] = [
  { id: "sharing", label: "Mon partage" },
  { id: "usage", label: "Mon utilisation" },
  { id: "guide", label: "Guide" },
];

function EditableName({
  displayName,
  hostname,
  onSave,
}: {
  displayName: string;
  hostname: string;
  onSave: (name: string) => void;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(displayName);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (editing) inputRef.current?.focus();
  }, [editing]);

  const commit = () => {
    setEditing(false);
    const trimmed = draft.trim();
    if (trimmed && trimmed !== displayName) {
      onSave(trimmed);
    } else {
      setDraft(displayName);
    }
  };

  if (!editing) {
    return (
      <button
        className="editable-name"
        onClick={() => {
          setDraft(displayName);
          setEditing(true);
        }}
        title="Cliquez pour renommer cette instance"
      >
        <span className="editable-name__display">{displayName}</span>
        <span className="editable-name__hostname">({hostname})</span>
        <span className="editable-name__icon">&#9998;</span>
      </button>
    );
  }

  return (
    <span className="editable-name editable-name--editing">
      <input
        ref={inputRef}
        className="editable-name__input"
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => {
          if (e.key === "Enter") commit();
          if (e.key === "Escape") {
            setDraft(displayName);
            setEditing(false);
          }
        }}
        maxLength={40}
      />
      <span className="editable-name__hostname">({hostname})</span>
    </span>
  );
}

export default function App() {
  const [activeTab, setActiveTab] = useState<Tab>("sharing");
  const [machineInfo, setMachineInfo] = useState<MachineInfo | null>(null);

  useEffect(() => {
    getMachineInfo()
      .then(setMachineInfo)
      .catch(() => {});
  }, []);

  const handleNameSave = async (name: string) => {
    const confirmed = await setDisplayName(name);
    setMachineInfo((prev) =>
      prev ? { ...prev, display_name: confirmed } : prev,
    );
  };

  return (
    <div className="app">
      <header className="app__header">
        <h1 className="app__title">PartaGPU</h1>
        {machineInfo && (
          <EditableName
            displayName={machineInfo.display_name}
            hostname={machineInfo.hostname}
            onSave={handleNameSave}
          />
        )}
      </header>

      <section className="app__room">
        <RoomSetup />
      </section>

      <nav className="app__nav">
        {TABS.map((tab) => (
          <button
            key={tab.id}
            className={`tab ${activeTab === tab.id ? "tab--active" : ""}`}
            onClick={() => setActiveTab(tab.id)}
          >
            {tab.label}
          </button>
        ))}
      </nav>

      <main className="app__main">
        {activeTab === "sharing" && <MySharing />}
        {activeTab === "usage" && <MyUsage />}
        {activeTab === "guide" && <Guide />}
      </main>
    </div>
  );
}
