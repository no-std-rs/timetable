// The browser supplies a clock, pointer input and RGBA presentation. Rust owns
// the device UI, catalogue, drafts and committed simulator settings.
export class DevicePreview {
  constructor(canvas, { snapshot, saved, error }) {
    Object.assign(this, { canvas, onSnapshot: snapshot, onSaved: saved, onError: error });
    this.context = canvas.getContext("2d");
    this.encoder = new TextEncoder();
    this.decoder = new TextDecoder();
    this.mode = 0;
    this.pending = null;
    this.generation = 0;
    this.visible = true;
    this.bindInput();
    this.observer = new IntersectionObserver(([entry]) => { this.visible = entry.isIntersecting; });
    this.observer.observe(canvas);
    this.timer = setInterval(() => {
      if (this.api && this.visible && !document.hidden) this.paint();
    }, 500);
  }
  now() { return BigInt(Math.floor(Date.now() / 1000)); }
  async load() {
    if (this.loading) return this.loading;
    this.loading = (async () => {
      try {
        const response = await fetch(new URL("./device-preview.wasm", import.meta.url));
        if (!response.ok) throw new Error("Preview download failed");
        const { instance } = await WebAssembly.instantiate(await response.arrayBuffer(), {});
        this.api = instance.exports;
        if (this.pending) this.configure(this.pending);
        return true;
      } catch {
        this.api = null;
        this.canvas.dataset.ready = "false";
        this.onError();
        return false;
      } finally { this.loading = null; }
    })();
    return this.loading;
  }
  configure({ groups, automatic, active }) {
    // No SSID/password/battery data crosses into WASM, including in error paths.
    this.pending = structuredClone({ groups, automatic, active });
    if (!this.api) return;
    const json = this.encoder.encode(JSON.stringify({ wifi_ssid: "Browser demo", wifi_password: "", automatic, groups }));
    if (json.length > this.api.preview_capacity()) { this.onError(); return; }
    new Uint8Array(this.api.memory.buffer, this.api.preview_input(), json.length).set(json);
    if (!this.api.preview_configure(json.length, this.now(), active)) { this.onError(); return; }
    this.generation++;
    this.api.preview_mode(this.mode, this.now());
    this.paint();
  }
  snapshot() {
    const length = this.api.preview_snapshot();
    if (!length) throw new Error("Preview state exceeds capacity");
    return JSON.parse(this.decoder.decode(new Uint8Array(this.api.memory.buffer, this.api.preview_output(), length)));
  }
  paint() {
    if (!this.api) return;
    try {
      if (!this.api.preview_render(this.now(), Math.floor(performance.now() / 500) % 8)) return;
      // Reacquire the view for every call; do not keep stale linear-memory views.
      const pixels = new Uint8ClampedArray(this.api.memory.buffer, this.api.preview_frame(), 240 * 240 * 4);
      this.context.putImageData(new ImageData(pixels, 240, 240), 0, 0);
      this.canvas.dataset.ready = "true";
      const state = this.snapshot();
      this.canvas.dataset.screen = ["board", "groups", "picks", "routes", "stops", "walk", "freshness"][state.screen];
      this.canvas.dataset.clock = state.clock_known ? "known" : "pending";
      this.onSnapshot(state);
    } catch { this.onError(); }
  }
  setMode(mode) {
    this.mode = Number(mode);
    this.generation++;
    if (this.api) { this.api.preview_mode(this.mode, this.now()); this.paint(); }
  }
  gesture(x, y, dx = 0, dy = 0, held = false) {
    if (!this.api) return;
    const action = this.api.preview_gesture(Math.round(x), Math.round(y), Math.round(dx), Math.round(dy), Number(held));
    this.paint();
    if (action >= 2) this.onSaved(this.snapshot());
    if (action === 1 || action === 3) {
      // Simulated network refresh, deliberately no timetable/NTP/Wi-Fi requests.
      const generation = ++this.generation;
      if (this.mode === 5) { this.api.preview_mode(5, this.now()); this.paint(); return; }
      setTimeout(() => {
        if (generation !== this.generation) return;
        this.api.preview_refresh(this.now());
        this.mode = 0;
        this.paint();
      }, 800);
    }
  }
  bindInput() {
    const point = event => {
      const box = this.canvas.getBoundingClientRect();
      return [(event.clientX - box.left) * 240 / box.width, (event.clientY - box.top) * 240 / box.height];
    };
    this.canvas.addEventListener("pointerdown", event => {
      if (event.button !== 0 || this.press) return;
      event.preventDefault();
      this.canvas.focus({ preventScroll: true });
      this.press = { id: event.pointerId, start: point(event), time: performance.now() };
      this.canvas.setPointerCapture(event.pointerId);
    });
    this.canvas.addEventListener("pointerup", event => {
      const press = this.press;
      if (!press || press.id !== event.pointerId) return;
      this.press = null;
      const [x, y] = point(event).map(value => Math.max(0, Math.min(239, Math.floor(value)))), [sx, sy] = press.start;
      this.gesture(x, y, x - sx, y - sy, performance.now() - press.time >= 700);
    });
    for (const name of ["pointercancel", "lostpointercapture"]) this.canvas.addEventListener(name, () => { this.press = null; });
    this.canvas.addEventListener("contextmenu", event => event.preventDefault());
    this.canvas.addEventListener("keydown", event => {
      const screen = this.api ? this.snapshot().screen : 0;
      const commands = {
        ArrowLeft: [120, 120, -80, 0], ArrowRight: [120, 120, 80, 0],
        ArrowUp: [120, 120, 0, -80], ArrowDown: [120, 120, 0, 80],
        Enter: screen === 5 ? [180, 200] : screen === 0 ? [120, 24] : [120, 70], Escape: [120, 24], Delete: [60, 200], e: [120, 120, 0, 0, true], E: [120, 120, 0, 0, true],
      };
      if (/^[1-4]$/.test(event.key) && this.api) {
        event.preventDefault();
        const index = Number(event.key) - 1;
        if (screen === 5) { this.walkRow = Math.min(index, 2); return; }
        const y = screen === 1 ? 52 + index * 36 + 10 : screen === 2 ? 55 + index * 32 + 10 : 58 + index * 32 + 10;
        this.gesture(120, y);
      } else if (["+", "-"].includes(event.key)) {
        event.preventDefault(); this.gesture(event.key === "+" ? 180 : 60, 80 + (this.walkRow ?? 0) * 38);
      } else if (commands[event.key]) {
        event.preventDefault();
        if (event.key === "Escape" && screen === 0) return;
        if (event.key === "Delete" && screen !== 5) return;
        this.gesture(...commands[event.key]);
      }
    });
  }
}
