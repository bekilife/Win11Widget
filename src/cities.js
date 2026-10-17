const { invoke } = window.__TAURI__.core;

const q = document.getElementById('q');
const results = document.getElementById('results');
const mine = document.getElementById('mine');
let cities = [];

const esc = (s) => String(s ?? '').replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));
const where = (c) => [c.admin, c.country].filter(Boolean).join(', ');

async function save() {
  await invoke('set_cities', { cities });
  renderMine();
}

function renderMine() {
  mine.innerHTML = cities.map((c, i) => `
    <li>
      <div class="n"><b>${esc(c.name)}</b>${i === 0 ? '<span class="main">ОСНОВНОЙ</span>' : ''}<small>${esc(where(c))}</small></div>
      ${i === 0 ? '<button class="star on" disabled title="Основной город">★</button>' : `<button class="star" data-main="${i}" title="Сделать основным">☆</button>`}
      <button data-up="${i}" ${i === 0 ? 'disabled' : ''} title="Выше">↑</button>
      <button data-down="${i}" ${i === cities.length - 1 ? 'disabled' : ''} title="Ниже">↓</button>
      <button data-del="${i}" title="Удалить">✕</button>
    </li>`).join('') || '<li><div class="n"><small>Пока пусто — найдите город выше</small></div></li>';
}

mine.addEventListener('click', (e) => {
  const b = e.target.closest('button');
  if (!b) return;
  const { up, down, del, main } = b.dataset;
  if (main !== undefined) cities.unshift(...cities.splice(+main, 1));
  if (up !== undefined) { const i = +up; [cities[i - 1], cities[i]] = [cities[i], cities[i - 1]]; }
  if (down !== undefined) { const i = +down; [cities[i + 1], cities[i]] = [cities[i], cities[i + 1]]; }
  if (del !== undefined) cities.splice(+del, 1);
  save();
});

let found = [];
let timer;
q.addEventListener('input', () => {
  clearTimeout(timer);
  timer = setTimeout(search, 300);
});

async function search() {
  const text = q.value.trim();
  if (text.length < 2) { results.innerHTML = ''; return; }
  try {
    const url = `https://geocoding-api.open-meteo.com/v1/search?name=${encodeURIComponent(text)}&count=8&language=ru&format=json`;
    const data = await (await fetch(url)).json();
    found = (data.results || []).map((r) => ({
      name: r.name, admin: r.admin1 || '', country: r.country || '', lat: r.latitude, lon: r.longitude,
    }));
    results.innerHTML = found.map((c, i) => `
      <li>
        <div class="n"><b>${esc(c.name)}</b><small>${esc(where(c))}</small></div>
        <button class="add" data-add="${i}">Добавить</button>
      </li>`).join('') || '<li><div class="n"><small>Ничего не найдено</small></div></li>';
  } catch {
    results.innerHTML = '<li><div class="n"><small>Нет соединения с сервисом поиска</small></div></li>';
  }
}

results.addEventListener('click', (e) => {
  const b = e.target.closest('[data-add]');
  if (!b) return;
  const c = found[+b.dataset.add];
  if (!cities.some((x) => Math.abs(x.lat - c.lat) < 0.01 && Math.abs(x.lon - c.lon) < 0.01)) cities.push(c);
  q.value = '';
  results.innerHTML = '';
  save();
  q.focus();
});

const reload = () => invoke('get_settings').then((s) => { cities = s.cities; renderMine(); });
reload();
// The main city can also be changed by clicking a city in the weather widget.
window.__TAURI__.event.listen('settings-changed', reload);
