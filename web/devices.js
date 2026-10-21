// Explicit board identities: firmware, flash geometry and preview travel together.
export const DEFAULT_DEVICE = "guition-c3";
export const DEVICES = Object.freeze({
  "guition-c3": Object.freeze({ name:"GUITION ESP32-2424S012-V1.0", chip:"ESP32-C3", chipId:5, flashSize:4194304, width:240, height:240, portrait:false, landscape:false, orientation:"round", path:"./" }),
  "waveshare-c6": Object.freeze({ name:"Waveshare ESP32-C6-Touch-LCD-1.47", chip:"ESP32-C6", chipId:13, flashSize:8388608, width:172, height:320, portrait:true, landscape:false, orientation:"portrait", path:"./devices/waveshare-c6/" }),
  "waveshare-c6-landscape": Object.freeze({ name:"Waveshare ESP32-C6-Touch-LCD-1.47", chip:"ESP32-C6", chipId:13, flashSize:8388608, width:320, height:172, portrait:false, landscape:true, orientation:"landscape", path:"./devices/waveshare-c6-landscape/" }),
});
export function device(id = DEFAULT_DEVICE) {
  const value = Object.hasOwn(DEVICES,id) ? DEVICES[id] : null;
  if (!value) throw new Error("Unknown display model.");
  return value;
}
export function releaseDevice(release) {
  const match = Object.values(DEVICES).find(d=>d.chip===release?.chip && d.name===release.board && d.flashSize===release.flash_size && d.orientation===(release.orientation ?? (release.chip==="ESP32-C6" ? "portrait" : "round")));
  if (!match) throw new Error("This firmware does not match the selected display or settings format.");
  return match;
}
