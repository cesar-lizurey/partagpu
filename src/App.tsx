import { useState, useEffect, useRef } from "react";
import { MySharing } from "./pages/MySharing";
import { MyUsage } from "./pages/MyUsage";
import { Fleet } from "./pages/Fleet";
import { Guide } from "./pages/Guide";
import { RoomSetup } from "./components/RoomSetup";
import { LanguageToggle } from "./components/LanguageToggle";
import { getMachineInfo, setDisplayName } from "./lib/api";
import type { MachineInfo } from "./lib/api";
import { useT } from "./lib/i18n";
import type { MessageKey } from "./lib/messages";
import { version as APP_VERSION } from "../package.json";
import "./styles.css";

type Tab = "sharing" | "usage" | "fleet" | "guide";

const TABS: { id: Tab; labelKey: MessageKey }[] = [
  { id: "sharing", labelKey: "tabs.sharing" },
  { id: "usage", labelKey: "tabs.usage" },
  { id: "fleet", labelKey: "tabs.fleet" },
  { id: "guide", labelKey: "tabs.guide" },
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
  const t = useT();
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
        title={t("app.rename_tooltip")}
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
  const t = useT();
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
        <h1 className="app__title">
          <img src="/favicon.png" alt="PartaGPU" className="app__logo" />
          PartaGPU
          <span className="app__version">v{APP_VERSION}</span>
        </h1>
        <div className="app__header-right">
          {machineInfo && (
            <EditableName
              displayName={machineInfo.display_name}
              hostname={machineInfo.hostname}
              onSave={handleNameSave}
            />
          )}
          <LanguageToggle />
        </div>
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
            {t(tab.labelKey)}
          </button>
        ))}
      </nav>

      <main className="app__main">
        {activeTab === "sharing" && <MySharing />}
        {activeTab === "usage" && <MyUsage />}
        {activeTab === "fleet" && <Fleet />}
        {activeTab === "guide" && <Guide />}
      </main>
    </div>
  );
}
