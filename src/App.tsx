import { getCurrentWindow } from "@tauri-apps/api/window";
import Pet from "./pet/Pet";
import Speech from "./bubble/Speech";
import UsagePanel from "./panel/UsagePanel";
import SettingsPanel from "./settings/SettingsPanel";
import CharacterStudio from "./studio/CharacterStudio";
import "./styles.css";

function App() {
  const label = getCurrentWindow().label;
  if (label === "bubble") return <Speech />;
  if (label === "panel") return <UsagePanel />;
  if (label === "settings") return <SettingsPanel />;
  if (label === "studio") return <CharacterStudio />;
  return <Pet />;
}

export default App;
