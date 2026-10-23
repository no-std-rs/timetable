// USB peer for the real pinned Rust/WASM transport, including old-adbd's
// repeated terminal CLSE. No mocked Client or shell protocol implementation.
export function installFakeADB() {
  const adb = { alternateSetting: 3, interfaceClass: 255, interfaceSubclass: 66, interfaceProtocol: 1, endpoints: [{ endpointNumber: 5, direction: "in", type: "bulk", packetSize: 64 }, { endpointNumber: 7, direction: "out", type: "bulk", packetSize: 64 }] };
  const configuration = { configurationValue: 2, interfaces: [{ interfaceNumber: 1, alternates: [adb] }] };
  const encode = text => new TextEncoder().encode(text);
  const decode = bytes => new TextDecoder().decode(bytes);
  const number = text => new DataView(encode(text).buffer).getUint32(0, true);
  let input = [], output = [], waiter = null, local = 0, remote = 0, streams = 0;
  window.usbPeer = { services: [], duplicates: 0 };
  function flush() { if (waiter && input.length) { const resolve = waiter; waiter = null; resolve(); } }
  function send(command, arg0, arg1, body = "") {
    const payload = encode(body), header = new Uint8Array(24), view = new DataView(header.buffer), id = number(command);
    [id, arg0, arg1, payload.length, payload.reduce((n, b) => n + b, 0), (id ^ 0xffffffff) >>> 0].forEach((v, i) => view.setUint32(i * 4, v, true));
    input.push(...header, ...payload); flush();
  }
  function receive(command, arg0, arg1, payload) {
    if (command === "CNXN") send("CNXN", 0x01000000, 4096, "device::\0");
    else if (command === "OPEN") {
      local = arg0; remote = 100 + ++streams;
      const service = decode(payload); window.usbPeer.services.push(service);
      const reply = window.usbPeer.replies?.shift();
      if(reply && !service.includes(reply.match))throw new Error(`Unexpected shell command: ${service}`);
      const answer = reply ? reply.body : service.includes("ro.build.version.sdk") ? String(window.fakeAdbSdk??22) : service.includes("ro.product.model") ? "Fire 7" : service.includes("ro.product.cpu.abi") ? "armeabi-v7a" : "";
      send("OKAY", remote, local); send("WRTE", remote, local, `${answer}\n\nTT_EXIT:${reply?.status??0}\n`);
    } else if (command === "OKAY") send("CLSE", remote, local);
    else if (command === "CLSE") { send("CLSE", remote, local); window.usbPeer.duplicates++; }
    else throw new Error(`Unexpected host ADB packet: ${command}`);
  }
  const device = {
    opened: false, configuration: null, configurations: [configuration],
    async open() { this.opened = true; input = []; output = []; },
    async selectConfiguration() { this.configuration = configuration; }, async claimInterface() {}, async selectAlternateInterface() {},
    async close() { this.opened = false; if (waiter) { const done = waiter; waiter = null; done(); } },
    async transferOut(endpoint, bytes) {
      if (endpoint !== 7) throw new Error("Wrong ADB OUT endpoint");
      output.push(...new Uint8Array(bytes.buffer, bytes.byteOffset, bytes.byteLength));
      while (output.length >= 24) {
        const header = Uint8Array.from(output.slice(0, 24)), view = new DataView(header.buffer), size = view.getUint32(12, true);
        if (output.length < 24 + size) break;
        const payload = Uint8Array.from(output.slice(24, 24 + size)); output.splice(0, 24 + size);
        receive(decode(header.slice(0, 4)), view.getUint32(4, true), view.getUint32(8, true), payload);
      }
      return { status: "ok", bytesWritten: bytes.byteLength };
    },
    async transferIn(endpoint, length) {
      if (endpoint !== 5) throw new Error("Wrong ADB IN endpoint");
      if (!input.length) await new Promise(resolve => waiter = resolve);
      if (!this.opened) throw new Error("USB closed");
      const count = Math.min(length, input.length, 37), bytes = new Uint8Array(count + 2);
      bytes.set(input.splice(0, count), 1);
      return { status: "ok", data: new DataView(bytes.buffer, 1, count) };
    },
  };
  Object.defineProperty(navigator, "usb", { configurable: true, value: { addEventListener() {}, async requestDevice() { return device; } } });
}
