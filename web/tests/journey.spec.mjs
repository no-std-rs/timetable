import { test, expect } from "@playwright/test";
import { readFile } from "node:fs/promises";
import { FeedLoader } from "../feed-loader.js";
const zip=await readFile(new URL("../../display/tests/fixtures/journey.zip",import.meta.url));
const catalog=await new FeedLoader().load(new Uint8Array(zip),{id:"mdb-982",provider:"Synthetic transit"});
async function prepare(page){
  await page.clock.setFixedTime(new Date(1792216800000));
  await page.route("**/catalog.json",r=>r.fulfill({json:catalog}));
  await page.route("**/latest.json",r=>r.fulfill({status:404}));
  await page.route("https://files.mobilitydatabase.org/mdb-982/latest.zip",r=>r.fulfill({body:zip,contentType:"application/zip"}));
  await page.goto("/");await expect(page.locator("#add-pick")).toBeEnabled();
  await page.locator("#group-mode").selectOption("journey");
  await expect(page.locator("#journey-depth")).toHaveValue("1");
  await expect(page.locator("#journey-slack")).toHaveValue("3");
}
async function add(page,kind,search,name){
  await page.locator(`#journey-${kind}-search`).fill(search);
  await page.locator(`#journey-${kind}-matches`).getByRole("button",{name,exact:true}).click();
}
test("journey settings preserve multiple walks, use one transfer by default and progressively explore deeper",async({page},info)=>{
  await prepare(page);
  await add(page,"origin","makow","Maków Polnych · A01");
  await add(page,"origin","further","Further stop · B01");
  await page.locator("#journey-origins input").nth(1).fill("12");
  await add(page,"target","bolechowo","Bolechowo-Os./Szkoła · Z01");
  await expect(page.locator("#picks")).toBeHidden();await expect(page.locator("#add-pick")).toBeHidden();
  await page.locator("#journey-editor details > summary").click();
  await page.locator("#journey-preview-depth").selectOption("4");
  await page.locator("#journey-preview").click();
  await expect(page.locator("#journey-preview-status")).toContainText("Search complete: checked up to 4 transfers");
  await expect(page.locator("#journey-preview-result")).toContainText("08:29");
  await expect(page.locator("#journey-preview-result")).toContainText("911");
  await expect(page.locator("#journey-preview-result")).toContainText("3 min transfer");
  // The optional browser depth must never silently replace the device setting.
  const saved=await page.evaluate(()=>JSON.parse(localStorage.getItem("timetable.settings.v1")));
  expect(saved.config.groups[0].journey).toMatchObject({max_transfers:1,transfer_minutes:3});
  expect(saved.config.groups[0].journey.origins.map(s=>s.walk)).toEqual([6,12]);
  await page.reload();await expect(page.locator("#group-mode")).toHaveValue("journey");
  await expect(page.locator("#journey-origins .journey-stop")).toHaveCount(2);
  await expect(page.locator("#journey-targets .journey-stop")).toHaveCount(1);
  expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
  await page.screenshot({path:`/tmp/timetable-journey-${info.project.name}.png`,fullPage:true});
});
test("changing journey settings invalidates a browser result and Polish controls keep their values",async({page})=>{
  await prepare(page);await add(page,"origin","makow","Maków Polnych · A01");await add(page,"target","bolechowo","Bolechowo-Os./Szkoła · Z01");
  await page.locator("#journey-editor details > summary").click();await page.locator("#journey-preview").click();
  await expect(page.locator("#journey-preview-result")).toContainText("08:29");
  await page.locator("#journey-slack").fill("4");await expect(page.locator("#journey-preview-result")).toBeEmpty();
  await page.locator(".masthead [data-language=pl]").click();
  await expect(page.locator("#group-mode option:checked")).toHaveText("Planowanie podróży (zaawansowane)");
  await expect(page.locator("#journey-slack")).toHaveValue("4");
  await expect(page.locator("#journey-targets")).toContainText("Bolechowo-Os./Szkoła");
});
