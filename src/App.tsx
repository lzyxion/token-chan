import { getCurrentWindow } from "@tauri-apps/api/window";
import Pet from "./pet/Pet";
import Bubble from "./bubble/Bubble";
import SettingsPanel from "./settings/SettingsPanel";
import "./styles.css";

function App() {
  const label = getCurrentWindow().label;
  if (label === "bubble") return <Bubble />;
  if (label === "settings") return <SettingsPanel />;
  return <Pet />;
}

export default App;
