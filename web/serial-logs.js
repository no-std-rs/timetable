// Browser-only passive USB reader. No flash writes or reset/control-line changes.
export class SerialLogs {
  constructor(serial, changed, limit = 65536) {
    this.serial = serial;
    this.changed = changed;
    this.limit = limit;
    this.text = "";
    this.state = "idle";
    this.generation = 0;
    this.port = null;
    this.reader = null;
    this.task = null;
    this.opening = null;
    this.closing = null;
  }
  append(value) {
    this.text = (this.text + value).slice(-this.limit);
    this.changed();
  }
  clear() { this.text = ""; this.changed(); }
  async connect() {
    if (["connecting", "connected", "disconnecting"].includes(this.state)) return;
    const generation = ++this.generation;
    this.state = "connecting";
    this.changed();
    const opening = this.open(generation);
    this.opening = opening;
    await opening;
    if (this.opening === opening) this.opening = null;
  }
  async open(generation) {
    let port;
    try {
      // Keep the port picker directly in the user's activation, before awaits.
      port = await this.serial.requestPort();
      if (generation !== this.generation) return;
      await port.open({ baudRate: 115200, bufferSize: 8192 });
      if (generation !== this.generation) { await port.close(); return; }
      this.port = port;
      this.state = "connected";
      this.changed();
      this.task = this.read(port, generation);
    } catch (error) {
      if (generation !== this.generation) return;
      this.state = error.name === "NotFoundError" ? "idle" : "error";
      this.changed();
    }
  }
  async read(port, generation) {
    const decoder = new TextDecoder();
    let failed = false;
    try {
      if (!port.readable) throw new Error("USB port is not readable");
      const reader = port.readable.getReader();
      this.reader = reader;
      try {
        while (generation === this.generation) {
          const { value, done } = await reader.read();
          if (done) break;
          if (value) this.append(decoder.decode(value, { stream: true }));
        }
        this.append(decoder.decode());
      } finally {
        reader.releaseLock();
        if (this.reader === reader) this.reader = null;
      }
    } catch { failed = true; }
    finally {
      try { await port.close(); } catch { /* A physically removed USB port is already closed. */ }
      if (this.port === port) this.port = null;
      if (generation === this.generation) {
        this.state = failed ? "error" : "disconnected";
        this.changed();
      }
    }
  }
  async disconnect() {
    if (this.state === "disconnecting") return this.closing;
    ++this.generation;
    this.state = "disconnecting";
    this.changed();
    this.closing = this.close();
    return this.closing;
  }
  async close() {
    const opening = this.opening;
    const reader = this.reader, task = this.task;
    try { await reader?.cancel(); } catch { /* USB removal can race cancellation. */ }
    // A late open owns the port until the stale generation closes it. Keep
    // installation and reconnect disabled until that cleanup has completed.
    if (opening) await opening;
    if (task) await task;
    this.task = null;
    this.state = "disconnected";
    this.changed();
  }
}
