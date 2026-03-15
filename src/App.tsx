import { BrowserRouter, Routes, Route } from "react-router-dom";
import { LocaleProvider } from "./hooks/useLocale";
import Layout from "./components/Layout";
import Dashboard from "./pages/Dashboard";
import AccountsPage from "./pages/AccountsPage";
import HubPage from "./pages/HubPage";
import ProxyPage from "./pages/ProxyPage";
import TokenStatsPage from "./pages/TokenStatsPage";
import MonitorPage from "./pages/MonitorPage";
import SettingsPage from "./pages/SettingsPage";

function App() {
  return (
    <LocaleProvider>
      <BrowserRouter>
        <Routes>
          <Route element={<Layout />}>
            <Route path="/" element={<Dashboard />} />
            <Route path="/accounts" element={<AccountsPage />} />
            <Route path="/hub" element={<HubPage />} />
            <Route path="/proxy" element={<ProxyPage />} />
            <Route path="/tokens" element={<TokenStatsPage />} />
            <Route path="/monitor" element={<MonitorPage />} />
            <Route path="/settings" element={<SettingsPage />} />
          </Route>
        </Routes>
      </BrowserRouter>
    </LocaleProvider>
  );
}

export default App;
