import {readFile} from "node:fs/promises";
import {test,expect} from "@playwright/test";
test.setTimeout(60000);
import {directory,archives,template} from "./world-fixtures.mjs";
async function prepare(page){
  await page.addInitScript(()=>Object.defineProperty(navigator,"serial",{value:{getPorts:async()=>[]},configurable:true}));
  await page.route("**/feeds.json",r=>r.fulfill({json:directory}));
  await page.route("**/latest.json",r=>r.fulfill({json:template.release}));
  await page.route("**/timetable-display.bin",r=>r.fulfill({body:Buffer.from(template.bytes),contentType:"application/octet-stream"}));
  await page.route("https://files.mobilitydatabase.org/**/latest.zip",r=>{
    const id=new URL(r.request().url()).pathname.split("/")[1];
    return r.fulfill({body:archives.get(id),contentType:"application/zip",headers:{"Access-Control-Allow-Origin":"*"}});
  });
  await page.goto("/");await expect(page.locator("#release-state")).toHaveText("Firmware verified",{timeout:15000});
  await page.locator("#network-panel summary").click();
}
async function select(page,id,country){
  await expect(page.locator(`#network-country option[value='${country}']`)).toHaveCount(1);
  await page.locator("#network-country").selectOption(country);await page.locator("#network-city").selectOption("*");
  await page.locator("#network-search").fill("");await page.locator("#network-feed").selectOption(id);await page.locator("#network-load").click();
  await expect(page.locator("#network-progress")).toHaveText("Network ready.",{timeout:15000});
  await expect(page.locator("#device-canvas")).toHaveAttribute("data-feed",id);
  await expect(page.locator("#network-progress")).toHaveText("Network ready.",{timeout:15000});
}
async function pick(page,stopName){
  await page.locator("#add-pick").click();await page.locator("#route-select").selectOption("0");
  await page.locator("#stop-search").fill(stopName);await page.locator("#stop-options [role=option]").first().click();
  const value=await page.locator("#direction-select option").evaluateAll(options=>options.find(o=>o.value.endsWith(":0"))?.value);await page.locator("#direction-select").selectOption(value);
  await page.locator("#save-pick").click();await expect(page.locator("#preview-stop")).toHaveText(stopName);
}
test("switching countries changes real routes, stops and WASM timezone, preserves per-network groups and reuses downloaded ZIPs",async({page})=>{
  const requests=[];page.on("request",r=>{if(r.url().endsWith("/latest.zip"))requests.push(r.url());});
  await prepare(page);await page.locator("#wifi-ssid").fill("private-home-ssid");await page.locator("#wifi-password").fill("private-home-password");
  await select(page,"demo-ny","US");await expect(page.locator("#schedule-timezone")).toHaveText("America/New_York");
  await page.locator("#group-name").fill("New York");await pick(page,"Broadway");
  await select(page,"demo-kolkata","IN");await expect(page.locator("#preview-line")).toHaveText("");await expect(page.locator("#device-canvas")).toHaveAttribute("data-timezone","Asia/Kolkata");
  await page.locator("#group-name").fill("India");await pick(page,"Park Street");
  await select(page,"demo-ny","US");await expect(page.locator("#group-name")).toHaveValue("New York");await expect(page.locator("#preview-stop")).toHaveText("Broadway");
  expect(requests.filter(url=>url.includes("demo-ny"))).toHaveLength(1);expect(requests.every(url=>!url.includes("private"))).toBeTruthy();
  await page.reload();await expect(page.locator("#device-canvas")).toHaveAttribute("data-feed","demo-ny");await expect(page.locator("#group-name")).toHaveValue("New York");
  await page.locator("#forget-settings").click();await page.reload();await expect(page.locator("#wifi-ssid")).toHaveValue("");await expect(page.locator("#device-canvas")).toHaveAttribute("data-feed","mdb-982");
  expect(await page.evaluate(()=>Object.keys(localStorage).filter(k=>k.startsWith("timetable.settings.v1")))).toEqual([]);
});
test("reopening saved network choices loads their route paths after switching networks and reloading",async({page})=>{
  await prepare(page);await select(page,"demo-ny","US");await pick(page,"Broadway");
  const savedDirection=await page.locator("#direction-select").inputValue();
  const reopen=async()=>{
    await page.locator(".pick-edit").first().click();
    await expect(page.locator("#route-select")).toHaveValue("0");
    await expect(page.locator("#route-map")).toBeVisible();
    await expect(page.locator("#path-select option")).toHaveCount(2);
    await expect(page.locator("#platform-hint")).toContainText("Terminal");
    await expect(page.locator("#direction-select")).toHaveValue(savedDirection);
    await page.locator("#cancel-pick").click();
  };
  await select(page,"demo-kolkata","IN");await select(page,"demo-ny","US");await reopen();
  await page.reload();await expect(page.locator("#device-canvas")).toHaveAttribute("data-feed","demo-ny");await reopen();
});
test("foreign-network USB installation contains that network’s checked ROM profile and keeps Wi-Fi solely in settings",async({page})=>{
  await prepare(page);await select(page,"demo-kolkata","IN");await pick(page,"Park Street");await page.locator("#wifi-ssid").fill("local-install-network");await page.locator("#wifi-password").fill("private-password");
  await page.locator("#prepare").click();await expect(page.locator("#install-area")).toBeVisible();
  const result=await page.locator("#installer").evaluate(async installer=>{
    const manifest=await(await fetch(installer.manifest)).json(),parts=manifest.builds[0].parts;
    const firmware=new Uint8Array(await(await fetch(parts[0].path)).arrayBuffer()),settings=new Uint8Array(await(await fetch(parts[1].path)).arrayBuffer());
    return {firmware:new TextDecoder().decode(firmware),settings:new TextDecoder().decode(settings.subarray(24,24+new DataView(settings.buffer).getUint32(12,true)))};
  });
  expect(result.firmware).toContain("demo-kolkata");expect(result.firmware).toContain("Asia/Kolkata");expect(result.firmware).toContain("Park Street");expect(result.firmware).not.toContain("private-password");expect(JSON.parse(result.settings).wifi_password).toBe("private-password");
  await page.locator("#wifi-ssid").fill("changed-network");await expect(page.locator("#install-area")).toBeHidden();
  await expect(page.locator("#digest")).toHaveText(`SHA-256 ${template.release.sha256}`);
});
test("Japanese stop names and platform codes get a flash glyph subset in the same Rust demo",async({page})=>{
  await prepare(page);await select(page,"demo-jp","JP");await pick(page,"東京駅");await expect(page.locator("#device-canvas")).toHaveAttribute("data-timezone","Asia/Tokyo");
  await page.locator("#wifi-ssid").fill("tokyo-local");await page.locator("#prepare").click();await expect(page.locator("#install-area")).toBeVisible();
  const glyphs=await page.locator("#installer").evaluate(async installer=>{
    const m=await(await fetch(installer.manifest)).json(),bytes=new Uint8Array(await(await fetch(m.builds[0].parts[0].path)).arrayBuffer());
    const signature=new TextEncoder().encode("TTGLY001"),offset=bytes.findIndex((_,i)=>signature.every((b,j)=>bytes[i+j]===b)),view=new DataView(bytes.buffer);
    return offset<0?[]:Array.from({length:view.getUint16(offset+8,true)},(_,i)=>String.fromCodePoint(view.getUint32(offset+10+24*i,true)));
  });expect(glyphs).toEqual(expect.arrayContaining(["東","京","駅","番","一"]));
  await page.locator(".masthead [data-language=pl]").click();await expect(page.locator("#network-load")).toHaveText("Wczytaj tę sieć");await expect(page.locator("#schedule-timezone")).toHaveText("Asia/Tokyo");await expect(page.locator("#network-scope")).toContainText("wszystkie linie");
});
for(const change of [{status:"inactive"},{status:"deprecated"},{mirror:"https://example.com/withdrawn.zip"}])test(`remembered unavailable networks stay unloaded: ${JSON.stringify(change)}`,async({page})=>{
  await prepare(page);await select(page,"demo-ny","US");await pick(page,"Broadway");
  const withdrawn=structuredClone(directory);Object.assign(withdrawn.feeds.find(f=>f.id==="demo-ny"),change);
  await page.route("**/feeds.json",r=>r.fulfill({json:withdrawn}));
  let downloads=0;await page.route("https://files.mobilitydatabase.org/demo-ny/latest.zip",r=>{downloads++;return r.abort();});
  await page.reload();
  await expect(page.locator("#network-progress")).toHaveText("Unavailable timetable");
  await expect(page.locator("#prepare")).toBeDisabled();await expect(page.locator("#add-pick")).toBeDisabled();
  expect(downloads).toBe(0);
  await page.locator("#network-panel summary").click();await expect(page.locator("#network-load")).toBeDisabled();
});
test("a cancelled network download cannot overwrite the next country’s catalogue or saved draft",async({page})=>{
  await prepare(page);
  await page.route("https://files.mobilitydatabase.org/demo-ny/latest.zip",async r=>{await new Promise(resolve=>setTimeout(resolve,1200));await r.fulfill({body:archives.get("demo-ny"),contentType:"application/zip"}).catch(()=>{});});
  await page.locator("#network-country").selectOption("US");await page.locator("#network-feed").selectOption("demo-ny");await page.locator("#network-load").click();
  await select(page,"demo-kolkata","IN");await page.locator("#group-name").fill("Keep India");await page.waitForTimeout(1400);
  await expect(page.locator("#device-canvas")).toHaveAttribute("data-feed","demo-kolkata");await expect(page.locator("#group-name")).toHaveValue("Keep India");await expect(page.locator("#network-progress")).toHaveText("Network ready.",{timeout:15000});
});

test("real raw-DEFLATE tables stream through the browser worker",async({page})=>{
  const bytes=await readFile(new URL("./fixtures/deflated.zip",import.meta.url));
  await prepare(page);
  await page.route("https://files.mobilitydatabase.org/demo-ny/latest.zip",r=>r.fulfill({body:bytes,contentType:"application/zip"}));
  await select(page,"demo-ny","US");await pick(page,"Broadway");
});
