import { searchText } from "./catalog-search.js";
export const DEFAULT_FEED = "mdb-982";
export const validFeedId = id => typeof id === "string" && /^[A-Za-z0-9_-]{1,80}$/.test(id);
export function mirrorURL(id) {
  if (!validFeedId(id)) throw new Error("Invalid timetable feed ID.");
  return `https://files.mobilitydatabase.org/${id}/latest.zip`;
}
export function directoryIndex(data, locale = "en") {
  if (!data || !Array.isArray(data.feeds) || data.feeds.length > 50000) throw new Error("Invalid feed directory.");
  const feeds = data.feeds;
  if (!feeds.every(f => f && typeof f.id === "string" && typeof f.country === "string" && typeof f.provider === "string" && typeof f.city === "string" && typeof f.region === "string" && typeof f.mirror === "string")) throw new Error("Invalid feed directory.");
  const regionNames = new Intl.DisplayNames([locale], { type: "region" }), compare = new Intl.Collator(locale, { numeric: true, sensitivity: "base" }).compare;
  const countryName = code => /^[A-Z]{2}$/.test(code) ? regionNames.of(code) : code;
  const locality = f => f.city || f.region;
  return {
    feeds, countryName, locality,
    countries: () => [...new Set(feeds.map(f => f.country))].sort((a,b) => compare(countryName(a),countryName(b))),
    cities: (country = null) => [...new Set(feeds.filter(f => country === null || f.country === country).map(locality))].sort(compare),
    search: (country = null, city = null, query = "") => {
      const terms = searchText(query).trim().split(/\s+/).filter(Boolean);
      return feeds.filter(f => (country === null || country === f.country) && (city === null || city === locality(f))
        && terms.every(t => searchText(`${f.id} ${countryName(f.country)} ${f.city} ${f.region} ${f.provider} ${f.name}`).includes(t)))
        .sort((a,b) => compare(locality(a),locality(b)) || compare(a.provider,b.provider) || compare(a.name,b.name) || compare(a.id,b.id));
    },
    available: f => validFeedId(f.id) && f.mirror === mirrorURL(f.id) && !["deprecated", "inactive"].includes(f.status),
  };
}
