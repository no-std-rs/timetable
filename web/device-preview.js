// The browser supplies a clock, pointer input and RGBA presentation. Rust owns
// the device UI, catalogue, drafts and committed simulator settings.
export class DevicePreview {
  constructor(canvas, { snapshot, saved, error }) {
    Object.assign(this, { canvas, onSnapshot: snapshot, onSaved: saved, onError: error });
    this.context = canvas.getContext("2d");
    this.encoder = new TextEncoder();
    this.decoder = new TextDecoder();
    this.mode = 0;
    this.width = 240; this.height = 240; this.portrait = false; this.landscape = false;
    this.pending = null;
    this.generation = 0;
    this.visible = true;
    this.suspend();
    this.bindInput();
    this.observer = new IntersectionObserver(([entry]) => { this.visible = entry.isIntersecting; });
    this.observer.observe(canvas);
    this.timer = setInterval(() => {
      if (this.api && this.visible && !document.hidden) this.paint();
    }, 500);
  }
  suspend() {
    this.configured = false;
    this.generation++;
    this.press = null;
    this.canvas.inert = true;
    this.canvas.dataset.ready = "false";
    this.canvas.setAttribute("aria-disabled", "true");
  }
  fail(reason) { this.suspend(); this.onError(reason); }
  now() { return BigInt(Math.floor(Date.now() / 1000)); }
  async load() {
    if (this.loading) return this.loading;
    this.loading = (async () => {
      try {
        const response = await fetch(new URL("./device-preview.wasm", import.meta.url), { cache: "no-store" });
        if (!response.ok) throw new Error("Preview download failed");
        const { instance } = await WebAssembly.instantiate(await response.arrayBuffer(), {});
        this.api = instance.exports;
        this.uploadedBundle = null;
        if (this.pending) this.configure(this.pending);
        return true;
      } catch {
        this.api = null;
        this.fail("download");
        return false;
      } finally { this.loading = null; }
    })();
    return this.loading;
  }
  clear() { this.suspend(); this.pending = null; }
  configure({ groups, automatic, active, bundle = null }) {
    // No SSID/password/battery data crosses into WASM, including in error paths.
    this.pending = { ...structuredClone({ groups, automatic, active }), bundle };
    // Cancel queued refreshes and reject stale interaction before validation.
    this.suspend();
    if (!this.api) return;
    if (!this.api.preview_device(this.landscape ? 2 : Number(this.portrait))) { this.fail("download"); return; }
    if (bundle && this.uploadedBundle !== bundle) {
      if (!this.api.preview_catalog_commit || bundle.length > this.api.preview_catalog_capacity()) { this.fail("settings"); return; }
      new Uint8Array(this.api.memory.buffer, this.api.preview_catalog_input(), bundle.length).set(bundle);
      if (!this.api.preview_catalog_commit(bundle.length)) { this.fail("settings"); return; }
      this.uploadedBundle = bundle;
    }
    const json = this.encoder.encode(JSON.stringify({ wifi_ssid: "Browser demo", wifi_password: "", automatic, groups }));
    if (json.length > this.api.preview_capacity()) { this.fail("settings"); return; }
    new Uint8Array(this.api.memory.buffer, this.api.preview_input(), json.length).set(json);
    if (!this.api.preview_configure(json.length, this.now(), active)) { this.fail("settings"); return; }
    this.configured = true;
    this.canvas.inert = false;
    this.canvas.setAttribute("aria-disabled", "false");
    this.api.preview_mode(this.mode, this.now());
    this.paint();
  }
  setDevice(board) {
    if (this.width === board.width && this.height === board.height) return;
    this.suspend();
    this.width = board.width; this.height = board.height; this.portrait = board.portrait; this.landscape = board.landscape;
    this.canvas.width = this.width; this.canvas.height = this.height;
  }
  snapshot() {
    const length = this.api.preview_snapshot();
    if (!length) throw new Error("Preview state exceeds capacity");
    return JSON.parse(this.decoder.decode(new Uint8Array(this.api.memory.buffer, this.api.preview_output(), length)));
  }
  paint() {
    if (!this.api || !this.configured) return;
    try {
      if (!this.api.preview_render(this.now(), Math.floor(performance.now() / 500) % 8)) return;
      // Reacquire the view for every call; do not keep stale linear-memory views.
      const pixels = new Uint8ClampedArray(this.api.memory.buffer, this.api.preview_frame(), this.width * this.height * 4);
      this.context.putImageData(new ImageData(pixels, this.width, this.height), 0, 0);
      this.canvas.dataset.ready = "true";
      const state = this.snapshot();
      this.canvas.dataset.screen = ["board", "groups", "picks", "routes", "stops", "walk", "freshness"][state.screen];
      this.canvas.dataset.orientation = this.landscape ? "landscape" : this.portrait ? "portrait" : "round";
      this.canvas.dataset.clock = state.clock_known ? "known" : "pending";
      this.canvas.dataset.feed = state.feed_id;
      this.canvas.dataset.timezone = state.timezone;
      this.onSnapshot(state);
    } catch { this.fail("render"); }
  }
  setMode(mode) {
    this.mode = Number(mode);
    this.generation++;
    if (this.api && this.configured) { this.api.preview_mode(this.mode, this.now()); this.paint(); }
  }
  gesture(x, y, dx = 0, dy = 0, held = false) {
    if (!this.api || !this.configured) return;
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
  keyboardGesture(x, y, dx = 0, dy = 0, held = false) {
    if (this.landscape) {
      const screen=this.snapshot().screen;
      let px=x*320/240, py;
      if (y<38) py=8;
      else if (screen===5) {
        if(y>=190)py=150;
        else if(y<63)py=44;
        else {const col=Math.max(0,Math.min(2,Math.floor((y-70)/38)));px=col*106+(x<120?28:78);py=100;}
      } else if(screen===1||screen===2) {
        if(y>=195)py=150;
        else {const index=Math.max(0,Math.floor((y-(screen===1?52:55))/(screen===1?36:32)));px=80+(index%2)*160;py=56+Math.floor(index/2)*48;}
      } else if(screen===3||screen===4) {
        if(y>=195)py=150;
        else {const index=Math.max(0,Math.floor((y-58)/32));if(index>=2)return;py=56+index*48;}
      } else py=y>=200?160:90;
      this.gesture(px,py,dx,dy,held);return;
    }
    if (!this.portrait) { this.gesture(x, y, dx, dy, held); return; }
    const screen = this.snapshot().screen;
    let py;
    if (y < 38) py = 16;
    else if (screen === 5) py = y >= 190 ? 288 : y < 63 ? 60 : 108 + Math.max(0, Math.floor((y - 70) / 38)) * 48;
    else if ([1,2,3,4].includes(screen)) py = y >= 195 ? 288 : 72 + Math.max(0, Math.floor((y - (screen === 1 ? 52 : screen === 2 ? 55 : 58)) / (screen === 1 ? 36 : 32))) * 48;
    else py = y >= 200 ? 288 : 160;
    this.gesture(x * 172 / 240, py, dx, dy, held);
  }
  bindInput() {
    const point = event => {
      const box = this.canvas.getBoundingClientRect();
      return [(event.clientX - box.left) * this.width / box.width, (event.clientY - box.top) * this.height / box.height];
    };
    this.canvas.addEventListener("pointerdown", event => {
      if (!this.configured || event.button !== 0 || this.press) return;
      event.preventDefault();
      this.canvas.focus({ preventScroll: true });
      this.press = { id: event.pointerId, start: point(event), time: performance.now() };
      this.canvas.setPointerCapture(event.pointerId);
    });
    this.canvas.addEventListener("pointerup", event => {
      const press = this.press;
      if (!press || press.id !== event.pointerId) return;
      this.press = null;
      const [x, y] = point(event).map((value, i) => Math.max(0, Math.min((i ? this.height : this.width) - 1, Math.floor(value)))), [sx, sy] = press.start;
      this.gesture(x, y, x - sx, y - sy, performance.now() - press.time >= 700);
    });
    for (const name of ["pointercancel", "lostpointercapture"]) this.canvas.addEventListener(name, () => { this.press = null; });
    this.canvas.addEventListener("contextmenu", event => event.preventDefault());
    this.canvas.addEventListener("keydown", event => {
      if (!this.configured) return;
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
        this.keyboardGesture(120, y);
      } else if (["+", "-"].includes(event.key)) {
        event.preventDefault(); this.keyboardGesture(event.key === "+" ? 180 : 60, 80 + (this.walkRow ?? 0) * 38);
      } else if (commands[event.key]) {
        event.preventDefault();
        if (event.key === "Escape" && screen === 0) return;
        if (event.key === "Delete" && screen !== 5) return;
        this.keyboardGesture(...commands[event.key]);
      }
    });
  }
}
