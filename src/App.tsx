import { getCurrentWindow } from "@tauri-apps/api/window";
import Pet from "./pet/Pet";
import Speech from "./bubble/Speech";
import UsagePanel from "./panel/UsagePanel";
import SettingsPanel from "./settings/SettingsPanel";
import "./styles.css";

function App() {
  const label = getCurrentWindow().label;
  if (label === "bubble") return <Speech />;
  if (label === "panel") return <UsagePanel />;
  if (label === "settings") return <SettingsPanel />;
  return <Pet />;
}

export default App;
