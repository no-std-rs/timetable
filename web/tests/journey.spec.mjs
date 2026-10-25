import { test, expect } from "@playwright/test";
import { readFile } from "node:fs/promises";
import { template } from "./world-fixtures.mjs";
import { FeedLoader } from "../feed-loader.js";
// 24-hour times keep stop labels exact; the page language stays English.
test.use({locale:"en-GB"});
const zip=await readFile(new URL("../../display/tests/fixtures/journey.zip",import.meta.url));
const catalog=await new FeedLoader().load(new Uint8Array(zip),{id:"mdb-982",provider:"Synthetic transit"});
async function prepare(page,fixtureZip=zip,fixtureCatalog=catalog,installable=false){
  await page.clock.setFixedTime(new Date(1792216800000));
  await page.route("**/catalog.json",r=>r.fulfill({json:fixtureCatalog}));
  await page.route("**/latest.json",r=>r.fulfill(installable?{json:template.release}:{status:404}));
  if(installable){
    await page.addInitScript(()=>Object.defineProperty(navigator,"serial",{value:{getPorts:async()=>[]},configurable:true}));
    await page.route("**/timetable-display.bin",r=>r.fulfill({body:Buffer.from(template.bytes),contentType:"application/octet-stream"}));
  }
  await page.route("https://files.mobilitydatabase.org/mdb-982/latest.zip",r=>r.fulfill({body:fixtureZip,contentType:"application/zip"}));
  await page.goto("/");await expect(page.locator("#add-pick")).toBeEnabled();
  await page.locator("#group-mode").selectOption("journey");
  await expect(page.locator("#journey-depth")).toHaveValue("1");
  await expect(page.locator("#journey-slack")).toHaveValue("3");
}
async function add(page,kind,search,name){
  await page.locator(`#journey-${kind}-search`).fill(search);
  await page.locator(`#journey-${kind}-matches`).getByRole("button",{name,exact:true}).click();
}
const shots=process.env.SCREENSHOT_DIR??"/tmp";
const routes=page=>page.locator("#journey-preview-result .journey-route");
async function find(page){
  await expect(page.locator("#journey-routes")).toBeVisible();
  await page.locator("#journey-preview").click();
  await expect(page.locator("#journey-preview-status")).toContainText("Search complete");
}
const alternativesZip=await readFile(new URL("../../display/tests/fixtures/journey-alternatives.zip",import.meta.url));
const alternativesCatalog=await new FeedLoader().load(new Uint8Array(alternativesZip),{id:"mdb-982",provider:"Synthetic transit"});
test("stops with no direct connection show every transfer route as stop dots and a full stop list",async({page})=>{
  await prepare(page,alternativesZip,alternativesCatalog);
  await add(page,"origin","makow","Maków Polnych · A01");
  await expect(page.locator("#journey-routes")).toBeHidden();
  await add(page,"target","bolechowo","Bolechowo-Os./Szkoła · Z01");
  await expect(page.locator("#journey-preview-status")).toContainText("downloads this network's timetable once");
  await find(page);
  const cards=routes(page);
  await expect(cards).toHaveCount(3);
  await expect(cards.nth(0)).toContainText("Via park");
  await expect(cards.nth(1)).toContainText("Via square");
  await expect(cards.nth(2)).toContainText("Alt");
  for(const card of await cards.all()){
    await expect(card).toContainText("Through stop · V01 · Pass through");
    await expect(card).toContainText("Uses your stops");
    await expect(card.locator(".journey-calls").nth(1).locator("li")).toHaveText([
      /^08:22.* · Interchange · X01 · Board here$/,/^08:25.* · Through stop · V01 · Pass through$/,/^08:30.* · Bolechowo-Os.\/Szkoła · Z01 · Alight here$/,
    ]);
  }
  // Every ride stop is a dot, including the non-boarding through stop.
  await expect(cards.nth(0).locator(".journey-route-map [role=button]")).toHaveCount(6);
  await expect(cards.nth(2).locator(".journey-route-map [role=button]")).toHaveCount(5);
  await expect(cards.nth(0).locator(".journey-route-map").getByRole("button",{name:/^Maków Polnych · A01 · 08:07.* · Your only starting stop$/})).toHaveAttribute("aria-disabled","true");
  expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
  await page.locator("#journey-routes").screenshot({path:`${shots}/timetable-route-alternatives-${page.viewportSize().width}.png`});
  // Once loaded, a language change re-renders and searches again automatically.
  await page.locator(".masthead [data-language=pl]").click();
  await expect(page.locator("#journey-preview-status")).toContainText("Wyszukiwanie zakończone");
  await expect(cards.first()).toContainText("Wsiądź");
  await expect(cards.first()).toContainText("Po drodze");
  await expect(page.locator("#journey-routes h3")).toHaveText("Trasy między Twoimi przystankami");
});
const untimedZip=await readFile(new URL("../../display/tests/fixtures/journey-untimed-alternatives.zip",import.meta.url));
const untimedCatalog=await new FeedLoader().load(new Uint8Array(untimedZip),{id:"mdb-982",provider:"Synthetic transit"});
test("untimed intermediate stops retain full platform codes and do not display invented times",async({page})=>{
  await prepare(page,untimedZip,untimedCatalog);
  await add(page,"origin","makow","Maków Polnych · A01");
  await add(page,"target","bolechowo","Bolechowo-Os./Szkoła · Z01");
  await find(page);
  const cards=routes(page);
  await expect(cards).toHaveCount(3);
  const park=cards.filter({hasText:"Via park"});
  await expect(park).toContainText("Time not provided · Via park · 番二三四五六七八九十 · Pass through");
  await expect(park.locator(".journey-route-map").getByRole("button",{name:/^Via park · 番二三四五六七八九十 · Time not provided/})).toHaveCount(1);
  await expect(park).toContainText("08:25");
  await expect(park).not.toContainText("Invalid Date");
  expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
  await page.locator(".masthead [data-language=pl]").click();
  await expect(page.locator("#journey-preview-status")).toContainText("Wyszukiwanie zakończone");
  await expect(park).toContainText("Brak godziny w rozkładzie · Via park · 番二三四五六七八九十");
});
test("journey settings preserve multiple walks, use one transfer by default and progressively explore deeper",async({page},info)=>{
  await prepare(page);
  await add(page,"origin","makow","Maków Polnych · A01");
  await add(page,"origin","further","Further stop · B01");
  await page.locator("#journey-origins input").nth(1).fill("12");
  await add(page,"target","bolechowo","Bolechowo-Os./Szkoła · Z01");
  await expect(page.locator("#picks")).toBeHidden();await expect(page.locator("#add-pick")).toBeHidden();
  await expect(page.locator("#journey-preview-depth")).toHaveValue("1");
  await page.locator("#journey-preview-depth").selectOption("4");
  await page.locator("#journey-preview").click();
  await expect(page.locator("#journey-preview-status")).toContainText("Search complete: checked up to 4 transfers");
  await expect(page.locator("#journey-preview-result")).toContainText("08:29");
  await expect(page.locator("#journey-preview-result")).toContainText("911");
  await expect(page.locator("#journey-preview-result")).toContainText("3 min change");
  // The optional browser depth must never silently replace the device setting.
  const saved=await page.evaluate(()=>JSON.parse(localStorage.getItem("timetable.settings.v1")));
  expect(saved.config.groups[0].journey).toMatchObject({max_transfers:1,transfer_minutes:3});
  expect(saved.config.groups[0].journey.origins.map(s=>s.walk)).toEqual([6,12]);
  await page.reload();await expect(page.locator("#group-mode")).toHaveValue("journey");
  await expect(page.locator("#journey-origins .journey-stop")).toHaveCount(2);
  await expect(page.locator("#journey-targets .journey-stop")).toHaveCount(1);
  expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
  await page.screenshot({path:`${shots}/timetable-journey-${info.project.name}.png`,fullPage:true});
});
test("changing journey settings refreshes loaded routes and Polish controls keep their values",async({page})=>{
  await prepare(page);await add(page,"origin","makow","Maków Polnych · A01");await add(page,"target","bolechowo","Bolechowo-Os./Szkoła · Z01");
  await find(page);
  await expect(page.locator("#journey-preview-result")).toContainText("Arrive 08:30");
  await page.locator("#journey-slack").fill("4");
  // With four minutes the 08:17 → 08:20 change is gone; the search reruns itself.
  await expect(page.locator("#journey-preview-result")).not.toContainText("Arrive 08:30");
  await expect(page.locator("#journey-preview-status")).toContainText("Search complete");
  await page.locator(".masthead [data-language=pl]").click();
  await expect(page.locator("#group-mode option:checked")).toHaveText("Planowanie podróży (zaawansowane)");
  await expect(page.locator("#journey-slack")).toHaveValue("4");
  await expect(page.locator("#journey-targets")).toContainText("Bolechowo-Os./Szkoła");
});

const walkableZip=await readFile(new URL("../../display/tests/fixtures/journey-walkable.zip",import.meta.url));
const walkableCatalog=await new FeedLoader().load(new Uint8Array(walkableZip),{id:"mdb-982",provider:"Synthetic transit"});
test("routes reveal a walkable stop with a direct bus, and tapping stops adds or removes them",async({page})=>{
  await prepare(page,walkableZip,walkableCatalog);
  await add(page,"origin","makow","Maków Polnych · 2 platforms");
  await add(page,"target","szkoła","Bolechowo-Os./Szkoła · S01");
  await find(page);
  const cards=routes(page);
  await expect(cards).toHaveCount(2);
  await expect(cards.nth(0).locator(".journey-route-head")).toContainText("Arrive 08:37");
  await expect(cards.nth(0)).toContainText("13 min walk to UAM Wydział Geografii · 348 08:13 UAM Wydział Geografii → 08:35 Bolechowo-Os./Wojska Polskiego · then 2 min walk");
  await expect(cards.nth(0)).toContainText("Add UAM Wydział Geografii (start, 13 min walk), Bolechowo-Os./Wojska Polskiego (target, 2 min walk after) to use this route");
  await expect(cards.nth(1)).toContainText("911 08:08 Maków Polnych (request stop) → 08:30 Biedrusko/Park · 6 min change · 397 08:36 Biedrusko/Park → 08:50 Bolechowo-Os./Szkoła (request stop)");
  await expect(cards.nth(1)).toContainText("Uses your stops");
  await expect(cards.nth(1).locator(".route-dot-request")).toHaveCount(3);
  await page.locator("#journey-routes").screenshot({path:`${shots}/timetable-walkable-routes-${page.viewportSize().width}.png`});
  const map=()=>routes(page).nth(0).locator(".journey-route-map");
  const start=map().getByRole("button",{name:"UAM Wydział Geografii · G01 · 08:13 · Add as a starting stop · about 13 min walk",exact:true});
  await start.focus();await expect(page.locator("#journey-route-hint")).toContainText("Add as a starting stop");
  await page.keyboard.press("Enter");
  await expect(page.locator("#journey-origins .journey-stop")).toHaveCount(2);
  await expect(page.locator("#journey-origins input").nth(1)).toHaveValue("13");
  // Loaded routes refresh automatically after every change.
  await expect(map().getByRole("button",{name:/^UAM Wydział Geografii · G01 · 08:13 · Your starting stop · tap to remove$/})).toHaveAttribute("aria-pressed","true");
  await map().getByRole("button",{name:"Bolechowo-Os./Wojska Polskiego · W01 · 08:35 · Add as a target · about 2 min walk after arrival",exact:true}).click();
  await expect(page.locator("#journey-targets .journey-stop")).toHaveCount(2);
  await expect(page.locator("#journey-targets input").nth(1)).toHaveValue("2");
  await expect(cards.nth(0)).toContainText("Uses your stops");
  await expect(cards).toHaveCount(2);
  const draft=await page.evaluate(()=>JSON.parse(localStorage.getItem("timetable.settings.v1")));
  expect(draft.config.groups[0].journey.origins.map(s=>[s.stop,s.walk])).toEqual([["A",6],["G",13]]);
  expect(draft.config.groups[0].journey.targets.map(s=>[s.stop,s.walk])).toEqual([["S",0],["W",2]]);
  await map().getByRole("button",{name:/^UAM Wydział Geografii · G01 · 08:13 · Your starting stop · tap to remove$/}).click();
  await expect(page.locator("#journey-origins .journey-stop")).toHaveCount(1);
  await page.locator("#journey-targets button",{hasText:"Remove"}).nth(1).click();
  await expect(cards.nth(0)).toContainText("Add UAM Wydział Geografii");
  await cards.nth(0).getByRole("button",{name:"Add these stops"}).click();
  await expect(page.locator("#journey-origins .journey-stop")).toHaveCount(2);
  await expect(page.locator("#journey-targets .journey-stop")).toHaveCount(2);
  expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
  await page.locator(".masthead [data-language=pl]").click();
  await expect(page.locator("#journey-preview-status")).toContainText("Wyszukiwanie zakończone");
  await expect(cards.nth(1)).toContainText("na żądanie");
  await expect(cards.nth(0)).toContainText("Korzysta z Twoich przystanków");
});

const groupedZip=await readFile(new URL("../../display/tests/fixtures/journey-places.zip",import.meta.url));
const groupedCatalog=await new FeedLoader().load(new Uint8Array(groupedZip),{id:"mdb-982",provider:"Synthetic transit"});
test("same-name platforms are one endpoint and routes use the faster platform",async({page})=>{
  await prepare(page,groupedZip,groupedCatalog);
  await page.locator("#journey-origin-search").fill("A02");
  await expect(page.locator("#journey-origin-matches button")).toHaveCount(1);
  await page.locator("#journey-origin-matches button").click();
  await expect(page.locator("#journey-origins .journey-stop")).toHaveCount(1);
  await expect(page.locator("#journey-origins small")).toHaveText("2 platforms");
  await page.locator("#journey-origin-search").fill("A02");
  await expect(page.locator("#journey-origin-matches button")).toHaveCount(0);
  await add(page,"target","Z02","Bolechowo-Os./Szkoła · 2 platforms");
  await page.locator("#journey-quick-minutes").fill("0");
  await find(page);
  await expect(page.locator("#journey-preview-result")).toContainText("08:22");
  await expect(page.locator("#journey-preview-result")).toContainText("A02");
  await expect(page.locator("#journey-preview-result")).toContainText("Z02");
  await page.reload();
  await expect(page.locator("#group-mode")).toHaveValue("journey");
  await expect(page.locator("#journey-origins .journey-stop")).toHaveCount(1);
  expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
});

const glyphZip=await readFile(new URL("../../display/tests/fixtures/journey-places-glyphs.zip",import.meta.url));
const glyphCatalog=await new FeedLoader().load(new Uint8Array(glyphZip),{id:"mdb-982",provider:"Synthetic transit"});
test("station-name search selects child platforms and installed profiles cover generated labels and unselected peer codes",async({page})=>{
  await prepare(page,glyphZip,glyphCatalog,true);
  await page.locator("#journey-origin-search").fill("番二三四五六七八九十");
  await expect(page.locator("#journey-origin-matches").getByRole("button",{name:"Central station · 2 platforms",exact:true})).toHaveCount(1);
  await add(page,"origin","central station","Central station · 2 platforms");
  await expect(page.locator("#journey-origins strong")).toHaveText("Central station");
  await add(page,"target","Z01","Bolechowo-Os./Szkoła · 2 platforms");
  await expect(page.locator("#release-state")).toHaveText("Firmware verified");
  await page.locator("#wifi-ssid").fill("journey-local");await page.locator("#prepare").click();
  await expect(page.locator("#install-area")).toBeVisible();
  await expect(page.locator("#network-scope")).toContainText("selected lines");
  const glyphs=await page.locator("#installer").evaluate(async installer=>{
    const manifest=await(await fetch(installer.manifest)).json(),bytes=new Uint8Array(await(await fetch(manifest.builds[0].parts[0].path)).arrayBuffer());
    const signature=new TextEncoder().encode("TTGLY001"),offset=bytes.findIndex((_,i)=>signature.every((b,j)=>bytes[i+j]===b)),view=new DataView(bytes.buffer);
    return offset<0?[]:Array.from({length:view.getUint16(offset+8,true)},(_,i)=>String.fromCodePoint(view.getUint32(offset+10+24*i,true)));
  });
  expect(glyphs).toEqual(expect.arrayContaining(["·","番","二","三","四","五","界"]));
  for(const hidden of ["一","耀"])expect(glyphs).not.toContain(hidden);
});


test("a short selectable anchor cannot hide an unsupported platform peer",async({page})=>{
  const incompatible=structuredClone(groupedCatalog);
  incompatible.stops.find(s=>s.id==="A2").id="x".repeat(33);
  await prepare(page,groupedZip,incompatible);
  await page.locator("#journey-origin-search").fill("maków");
  const choice=page.locator("#journey-origin-matches").getByRole("button",{name:"Maków Polnych · 2 platforms",exact:true});
  await expect(choice).toBeDisabled();
  await expect(choice).toHaveAttribute("title",/32-byte/);
  await page.locator("#journey-target-search").fill("maków");
  await expect(page.locator("#journey-target-matches").getByRole("button",{name:"Maków Polnych · 2 platforms",exact:true})).toBeDisabled();
});
