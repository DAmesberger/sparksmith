const search = document.querySelector("#search");
const groups = [...document.querySelectorAll("[data-group]")];
const count = document.querySelector("#count");
function filter() {
  const words = search.value.trim().toLocaleLowerCase().split(/\s+/).filter(Boolean);
  let total = 0;
  for (const group of groups) {
    let matched = 0;
    for (const row of group.querySelectorAll("[data-terms]")) {
      row.hidden = !words.every(word => row.dataset.terms.includes(word));
      if (!row.hidden) matched++;
    }
    group.hidden = !matched;
    total += matched;
  }
  count.textContent = words.length ? total + (total === 1 ? " topic" : " topics") : "";
}
search.addEventListener("input", filter);
search.addEventListener("keydown", event => {
  if (event.key === "Enter") document.querySelector("[data-group]:not([hidden]) li:not([hidden]) a")?.focus();
});
