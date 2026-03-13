import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";

interface Message {
  role: "user" | "assistant";
  content: string;
}

function App() {
  const [messages, setMessages] = useState<Message[]>([]);
  const [input, setInput] = useState("");
  const [apiKey, setApiKey] = useState("");
  const [initialized, setInitialized] = useState(false);
  const [loading, setLoading] = useState(false);

  async function handleInit() {
    if (!apiKey.trim()) return;
    try {
      await invoke("initialize_agent", { apiKey });
      setInitialized(true);
    } catch (e) {
      console.error(e);
    }
  }

  async function handleSend() {
    if (!input.trim() || loading) return;
    const userMessage = input.trim();
    setInput("");
    setMessages((prev) => [...prev, { role: "user", content: userMessage }]);
    setLoading(true);
    try {
      const response = await invoke<string>("chat", { message: userMessage });
      setMessages((prev) => [...prev, { role: "assistant", content: response }]);
    } catch (e) {
      setMessages((prev) => [
        ...prev,
        { role: "assistant", content: `Error: ${e}` },
      ]);
    } finally {
      setLoading(false);
    }
  }

  if (!initialized) {
    return (
      <div className="setup">
        <h1>OpenComputer</h1>
        <p>Your personal AI assistant</p>
        <input
          type="password"
          placeholder="Enter your Anthropic API key"
          value={apiKey}
          onChange={(e) => setApiKey(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && handleInit()}
        />
        <button onClick={handleInit}>Get Started</button>
      </div>
    );
  }

  return (
    <div className="chat">
      <div className="messages">
        {messages.length === 0 && (
          <div className="welcome">
            <h2>How can I help you today?</h2>
          </div>
        )}
        {messages.map((msg, i) => (
          <div key={i} className={`message ${msg.role}`}>
            <div className="bubble">{msg.content}</div>
          </div>
        ))}
        {loading && (
          <div className="message assistant">
            <div className="bubble loading">...</div>
          </div>
        )}
      </div>
      <div className="input-bar">
        <input
          type="text"
          placeholder="Ask anything..."
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && handleSend()}
        />
        <button onClick={handleSend} disabled={loading}>
          Send
        </button>
      </div>
    </div>
  );
}

export default App;
