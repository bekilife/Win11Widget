const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const appWindow = window.__TAURI__.window.getCurrentWindow();

const card = document.getElementById('card');
const kind = new URLSearchParams(location.search).get('w');
const WEATHER_EVERY = 15 * 60 * 1000;

// Drag anywhere; right-click opens the native widget menu.
document.addEventListener('mousedown', (e) => {
  if (e.button === 0 && !e.target.closest('[data-nodrag]')) appWindow.startDragging();
});
document.addEventListener('contextmenu', (e) => {
  e.preventDefault();
  invoke('widget_menu');
});

// ---------------- glass ----------------
// Widgets sit on the desktop layer, so only the wallpaper is ever behind them.
// Drawing the blurred piece of wallpaper under the window looks like real frosted glass.
const GLASS_SCALE = 0.25; // work on a quarter-resolution copy; it gets blurred anyway
const GLASS_BLUR = 34;    // CSS px

const glassHost = document.getElementById('glass');
const glassCanvas = document.createElement('canvas');
let glassImg = null;
let glassCover = null; // wallpaper fitted ("Fill") to the monitor, at GLASS_SCALE

async function paintGlass() {
  if (!glassImg) return;
  const mon = await window.__TAURI__.window.currentMonitor();
  if (!mon) return;
  const pos = await appWindow.outerPosition();
  const sf = await appWindow.scaleFactor();
  const k = GLASS_SCALE;
  const mw = Math.round(mon.size.width * k), mh = Math.round(mon.size.height * k);

  if (!glassCover || glassCover.width !== mw || glassCover.height !== mh) {
    glassCover = document.createElement('canvas');
    glassCover.width = mw;
    glassCover.height = mh;
    const fitK = Math.max(mw / glassImg.naturalWidth, mh / glassImg.naturalHeight);
    const dw = glassImg.naturalWidth * fitK, dh = glassImg.naturalHeight * fitK;
    glassCover.getContext('2d').drawImage(glassImg, (mw - dw) / 2, (mh - dh) / 2, dw, dh);
  }

  // Extra margin so the blur has real pixels to sample at the edges.
  const blur = GLASS_BLUR * sf * k;
  const m = Math.ceil(blur * 2);
  const w = Math.ceil(innerWidth * sf * k) + 2 * m;
  const h = Math.ceil(innerHeight * sf * k) + 2 * m;
  glassCanvas.width = w;
  glassCanvas.height = h;
  const ctx = glassCanvas.getContext('2d');
  ctx.filter = `blur(${blur}px) saturate(1.5)`;
  ctx.drawImage(glassCover, -(pos.x - mon.position.x) * k + m, -(pos.y - mon.position.y) * k + m);

  const css = 1 / (sf * k);
  Object.assign(glassCanvas.style, {
    left: `${-m * css}px`, top: `${-m * css}px`, width: `${w * css}px`, height: `${h * css}px`,
  });
}

let glassQueued = false;
function scheduleGlass() {
  if (glassQueued) return;
  glassQueued = true;
  requestAnimationFrame(() => { glassQueued = false; paintGlass(); });
}

/** (Re)load the wallpaper; without one the card keeps its solid background. */
async function loadGlass() {
  const url = await invoke('wallpaper');
  if (!url) return;
  const img = new Image();
  img.src = url;
  await img.decode();
  glassImg = img;
  glassCover = null;
  glassHost.replaceChildren(glassCanvas);
  document.body.classList.add('has-glass');
  scheduleGlass();
}

appWindow.onMoved(scheduleGlass);
appWindow.onResized(scheduleGlass);
loadGlass().catch((e) => console.error('glass', e));
listen('refresh', () => loadGlass().catch(() => {}));


const pad = (n) => String(n).padStart(2, '0');

/** Size the window to its content (auto-height widgets). */
const fit = () => card.firstElementChild && invoke('fit_height', { height: card.firstElementChild.offsetHeight });
const fitObserver = new ResizeObserver(() => fit());
/** Call after replacing card content: keeps the window as tall as the content. */
function observeFit() {
  fitObserver.disconnect();
  if (card.firstElementChild) fitObserver.observe(card.firstElementChild);
}

/** Runs fn now and then at the start of every following second/minute. */
function everyTick(ms, fn) {
  fn();
  setTimeout(() => { fn(); setInterval(fn, ms); }, ms - (Date.now() % ms) + 15);
}

// ---------------- clock (digital) ----------------
function clock() {
  card.innerHTML = `
    <div class="clock">
      <div class="wd red" id="wd"></div>
      <div class="time" id="tm"></div>
      <div class="date" id="dt"></div>
      <div class="year" id="yr"></div>
    </div>`;
  const $ = (id) => document.getElementById(id);
  everyTick(1000, () => {
    const d = new Date();
    $('tm').textContent = `${pad(d.getHours())}:${pad(d.getMinutes())}`;
    $('wd').textContent = d.toLocaleDateString('ru-RU', { weekday: 'long' }).toUpperCase();
    $('dt').textContent = d.toLocaleDateString('ru-RU', { day: 'numeric', month: 'long' });
    $('yr').textContent = String(d.getFullYear());
  });
}

// ---------------- calendar ----------------
function calendar() {
  let shown = '';
  const render = () => {
    const now = new Date();
    const key = now.toDateString();
    if (key === shown) return;
    shown = key;
    const y = now.getFullYear(), mo = now.getMonth();
    const lead = (new Date(y, mo, 1).getDay() + 6) % 7; // Monday-first
    const days = new Date(y, mo + 1, 0).getDate();
    let cells = ['П', 'В', 'С', 'Ч', 'П', 'С', 'В'].map((h) => `<span class="h">${h}</span>`).join('');
    cells += '<span></span>'.repeat(lead);
    for (let d = 1; d <= days; d++) {
      const col = (lead + d - 1) % 7;
      const cls = d === now.getDate() ? 'today' : col >= 5 ? 'we' : '';
      cells += `<span class="${cls}">${d}</span>`;
    }
    const month = new Intl.DateTimeFormat('ru-RU', { month: 'long' }).format(now);
    card.innerHTML = `<div class="cal"><div class="month red">${month.toUpperCase()}</div><div class="grid">${cells}</div></div>`;
  };
  everyTick(60000, render);
}

// ---------------- system ----------------
const GiB = 1024 ** 3;
const size = (b) =>
  b >= 1024 * GiB ? `${(b / 1024 / GiB).toFixed(1)} ТБ` : `${b >= 100 * GiB ? Math.round(b / GiB) : (b / GiB).toFixed(1)} ГБ`;
const speed = (bps) => (bps >= 1024 * 1024 ? `${(bps / 1024 / 1024).toFixed(1)} МБ/с` : `${Math.round(bps / 1024)} КБ/с`);
function uptimeText(sec) {
  const d = Math.floor(sec / 86400), h = Math.floor((sec % 86400) / 3600), m = Math.floor((sec % 3600) / 60);
  return d ? `${d} д ${h} ч` : h ? `${h} ч ${m} мин` : `${m} мин`;
}
/** Monochrome UI: white while calm, orange/red only as a warning. */
const WHITE = '#ffffff';
/** Calm colour until the resource gets tight. */
const loadColor = (pct, calm) => (pct >= 95 ? '#ff453a' : pct >= 85 ? '#ff9f0a' : calm);
const tempColor = (t) => (t >= 85 ? '#ff453a' : t >= 70 ? '#ff9f0a' : WHITE);

const MINI_ICON = {
  cpu: '<svg viewBox="0 0 24 24"><rect x="6" y="6" width="12" height="12" rx="2.5" fill="none" stroke="currentColor" stroke-width="1.7"/><rect x="9.5" y="9.5" width="5" height="5" rx="1" fill="currentColor"/><path d="M9 3v3M15 3v3M9 18v3M15 18v3M3 9h3M3 15h3M18 9h3M18 15h3" stroke="currentColor" stroke-width="1.7" stroke-linecap="round"/></svg>',
  ram: '<svg viewBox="0 0 24 24"><rect x="3" y="7" width="18" height="10" rx="2" fill="none" stroke="currentColor" stroke-width="1.7"/><path d="M7 10v4M11 10v4M15 10v4M6 17v2.5M10 17v2.5M14 17v2.5M18 17v2.5" stroke="currentColor" stroke-width="1.7" stroke-linecap="round"/></svg>',
  gpu: '<svg viewBox="0 0 24 24"><rect x="2.5" y="6" width="19" height="11" rx="2" fill="none" stroke="currentColor" stroke-width="1.7"/><circle cx="9" cy="11.5" r="3" fill="none" stroke="currentColor" stroke-width="1.7"/><path d="M15 9.5h3.5M15 13.5h3.5M5 17v2.5" stroke="currentColor" stroke-width="1.7" stroke-linecap="round"/></svg>',
  disk: '<svg viewBox="0 0 24 24"><rect x="3.5" y="5" width="17" height="14" rx="2.5" fill="none" stroke="currentColor" stroke-width="1.7"/><circle cx="16.5" cy="15" r="1.3" fill="currentColor"/><path d="M7 15h5" stroke="currentColor" stroke-width="1.7" stroke-linecap="round"/></svg>',
};
const MINI_H = 150; // compact height, keep in sync with the "system" Def in lib.rs

function systemWidget() {
  card.classList.add('sys-card');
  const R = 22, C = 2 * Math.PI * R;
  const MR = 27, MC = 2 * Math.PI * MR;
  const mini = (id, icon, label) => `
    <div class="mc">
      <div class="mring">
        <svg viewBox="0 0 64 64">
          <circle cx="32" cy="32" r="${MR}" id="${id}-bg"/>
          <circle cx="32" cy="32" r="${MR}" id="${id}-fg" stroke-dasharray="${MC}" stroke-dashoffset="${MC}"/>
        </svg>
        <div class="ico" id="${id}-i">${icon}</div>
      </div>
      <div class="ml" id="${id}-l">${label}</div>
      <div class="mp" id="${id}-p">–</div>
    </div>`;
  const block = (id, title, temp = false) => `
    <div class="blk">
      <div class="ring">
        <svg viewBox="0 0 52 52">
          <circle cx="26" cy="26" r="${R}" id="${id}-bg"/>
          <circle cx="26" cy="26" r="${R}" id="${id}-fg" stroke-dasharray="${C}" stroke-dashoffset="${C}"/>
        </svg>
        <div class="v" id="${id}-v">–</div>
      </div>
      <div class="txt">
        <div class="k">${title}</div>
        <div class="n" id="${id}-n"></div>
        <div class="s" id="${id}-s"></div>
      </div>
      ${temp ? `<div class="temp" id="${id}-t" title="Температура"></div>` : ''}
    </div>`;
  card.innerHTML = `
    <div class="sysmini">${mini('m-cpu', MINI_ICON.cpu, 'Процессор')}${mini('m-ram', MINI_ICON.ram, 'Память')}${mini('m-x', MINI_ICON.gpu, 'Видеокарта')}</div>
    <div class="sysf">
      <div class="hdr"><span class="ttl">Система</span><span class="up" id="uptime"></span></div>
      <div class="main">${block('cpu', 'Процессор')}${block('ram', 'Память')}<div id="gpus" class="main"></div></div>
      <div class="net">
        <span class="k">Сеть</span>
        <span class="dn">↓ <b id="net-dn">–</b></span>
        <span class="upl">↑ <b id="net-up">–</b></span>
      </div>
      <div class="dh">Диски</div>
      <div id="disks"></div>
    </div>`;
  const $ = (id) => document.getElementById(id);
  const ring = (id, pct, color) => {
    const fg = $(`${id}-fg`);
    fg.style.stroke = color;
    fg.style.strokeDashoffset = C * (1 - Math.max(0, Math.min(100, pct)) / 100);
    $(`${id}-bg`).style.stroke = color + '33';
    $(`${id}-v`).textContent = `${Math.round(pct)}%`;
  };

  const miniRing = (id, pct, color) => {
    const fg = $(`${id}-fg`);
    fg.style.stroke = color;
    fg.style.strokeDashoffset = MC * (1 - Math.max(0, Math.min(100, pct)) / 100);
    $(`${id}-bg`).style.stroke = color + '2e';
    $(`${id}-i`).style.color = color;
    $(`${id}-p`).textContent = `${Math.round(pct)}%`;
  };

  // Hover: show the full version over the neighbours; leave: back to compact.
  const full = card.querySelector('.sysf');
  let expanded = false, openT = 0, closeT = 0;
  const sizeObserver = new ResizeObserver(() => {
    if (expanded) invoke('system_expand', { expanded: true, height: full.offsetHeight });
  });
  const expand = () => {
    if (expanded) return;
    expanded = true;
    card.classList.add('expanded');
    invoke('system_expand', { expanded: true, height: full.offsetHeight });
    sizeObserver.observe(full);
  };
  const collapse = () => {
    if (!expanded) return;
    expanded = false;
    sizeObserver.disconnect();
    card.classList.remove('expanded');
    invoke('system_expand', { expanded: false, height: MINI_H });
  };
  document.documentElement.addEventListener('mouseenter', () => {
    clearTimeout(closeT);
    openT = setTimeout(expand, 250);
  });
  document.documentElement.addEventListener('mouseleave', () => {
    clearTimeout(openT);
    closeT = setTimeout(collapse, 300);
  });

  let gpuKey = '';
  const renderGpus = (gpus) => {
    const key = gpus.map((g) => g.name).join('|');
    if (key !== gpuKey) {
      gpuKey = key;
      $('gpus').innerHTML = gpus.map((g, i) => block(`g${i}`, 'Видеокарта', true)).join('');
    }
    gpus.forEach((g, i) => {
      ring(`g${i}`, g.load, loadColor(g.load, WHITE));
      $(`g${i}-n`).textContent = g.name;
      const parts = [`VRAM ${(g.memUsed / GiB).toFixed(1)} из ${Math.round(g.memTotal / GiB)} ГБ`];
      if (g.power != null) parts.push(`${Math.round(g.power)} Вт`);
      if (g.fan) parts.push(`вент. ${g.fan}%`);
      $(`g${i}-s`).textContent = parts.join(' · ');
      const t = $(`g${i}-t`);
      t.textContent = g.temp != null ? `${g.temp}°` : '';
      t.style.color = g.temp != null ? tempColor(g.temp) : '';
    });
  };

  let diskKey = '';
  const renderDisks = (disks) => {
    const key = disks.map((d) => d.mount).join('|');
    if (key !== diskKey) {
      diskKey = key;
      $('disks').innerHTML = disks.map((d, i) => `
        <div class="disk">
          <div class="dl">
            <b>${d.mount}</b>
            <span class="lab">${d.label || (d.removable ? 'Съёмный диск' : 'Локальный диск')}</span>
            ${d.removable ? '<span class="tag">USB</span>' : d.kind ? `<span class="tag">${d.kind}</span>` : ''}
            <span class="pct" id="d${i}-p"></span>
          </div>
          <div class="bar"><i id="d${i}-b"></i></div>
          <div class="ds" id="d${i}-s"></div>
        </div>`).join('') || '<div class="ds">Диски не найдены</div>';
    }
    disks.forEach((d, i) => {
      const pct = ((d.total - d.available) / d.total) * 100;
      $(`d${i}-p`).textContent = `${Math.round(pct)}%`;
      const bar = $(`d${i}-b`);
      bar.style.width = `${pct}%`;
      bar.style.background = loadColor(pct, WHITE);
      $(`d${i}-s`).textContent = `${size(d.available)} свободно из ${size(d.total)}`;
    });
  };

  const tick = async () => {
    try {
      const s = await invoke('system_stats');
      ring('cpu', s.cpu, loadColor(s.cpu, WHITE));
      $('cpu-n').textContent = s.cpuName || 'Процессор';
      $('cpu-n').title = s.cpuName;
      $('cpu-s').textContent = `${s.threads} потоков${s.cpuFreq ? ` · ${(s.cpuFreq / 1000).toFixed(1)} ГГц` : ''}`;
      const ram = (s.ramUsed / s.ramTotal) * 100;
      ring('ram', ram, loadColor(ram, WHITE));
      $('ram-n').textContent = `${(s.ramUsed / GiB).toFixed(1)} из ${Math.round(s.ramTotal / GiB)} ГБ`;
      $('ram-s').textContent = `Свободно ${((s.ramTotal - s.ramUsed) / GiB).toFixed(1)} ГБ`;
      $('net-dn').textContent = speed(s.netDown);
      $('net-up').textContent = speed(s.netUp);
      $('uptime').textContent = `Работает ${uptimeText(s.uptime)}`;
      renderGpus(s.gpus || []);
      renderDisks(s.disks);
      miniRing('m-cpu', s.cpu, loadColor(s.cpu, WHITE));
      miniRing('m-ram', ram, loadColor(ram, WHITE));
      const g = (s.gpus || [])[0];
      if (g) {
        miniRing('m-x', g.load, loadColor(g.load, WHITE));
      } else {
        // No NVIDIA GPU: show the system drive instead.
        const c = s.disks.find((d) => d.mount.toUpperCase().startsWith('C')) || s.disks[0];
        if (c) {
          const pct = ((c.total - c.available) / c.total) * 100;
          $('m-x-l').textContent = `Диск ${c.mount}`;
          $('m-x-i').innerHTML = MINI_ICON.disk;
          miniRing('m-x', pct, loadColor(pct, WHITE));
        }
      }
    } catch (e) { console.error(e); }
  };
  tick();
  setInterval(tick, 2000);
}

// ---------------- now playing ----------------
const ICON = {
  prev: '<svg viewBox="0 0 24 24"><path d="M6 5h2v14H6zM20 5v14L9.5 12z" fill="currentColor"/></svg>',
  next: '<svg viewBox="0 0 24 24"><path d="M16 5h2v14h-2zM4 5v14l10.5-7z" fill="currentColor"/></svg>',
  play: '<svg viewBox="0 0 24 24"><path d="M7 4.5v15l13-7.5z" fill="currentColor"/></svg>',
  pause: '<svg viewBox="0 0 24 24"><path d="M6 4.5h4.2v15H6zM13.8 4.5H18v15h-4.2z" fill="currentColor"/></svg>',
  note: '<svg viewBox="0 0 24 24"><path d="M9 17.5a2.5 2.5 0 1 1-2.5-2.5c.36 0 .7.07 1 .2V5.5l11-2.5v12a2.5 2.5 0 1 1-2.5-2.5c.36 0 .7.07 1 .2V7.3l-8 1.8z" fill="currentColor"/></svg>',
};
const mmss = (sec) => {
  const s = Math.max(0, Math.floor(sec));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
};

function media() {
  card.innerHTML = `
    <div class="np">
      <div class="art" id="art">${ICON.note}</div>
      <div class="meta">
        <div class="src" id="src"></div>
        <div class="title" id="title">Ничего не играет</div>
        <div class="artist" id="artist">Включите музыку в любом плеере</div>
        <div class="prog" id="prog">
          <div class="bar"><i id="bar"></i></div>
          <div class="times"><span id="pos">0:00</span><span id="dur">0:00</span></div>
        </div>
        <div class="ctl" data-nodrag>
          <button id="prev" title="Предыдущий">${ICON.prev}</button>
          <button id="toggle" class="main" title="Воспроизвести / пауза">${ICON.play}</button>
          <button id="next" title="Следующий">${ICON.next}</button>
        </div>
      </div>
    </div>`;
  const $ = (id) => document.getElementById(id);
  let st = null;
  let artKey = null;

  const paintProgress = () => {
    if (!st || !st.active || !st.duration) {
      $('prog').classList.add('off');
      return;
    }
    $('prog').classList.remove('off');
    let pos = st.position;
    if (st.playing && st.updatedAt) pos += (Date.now() - st.updatedAt) / 1000;
    pos = Math.min(pos, st.duration);
    $('bar').style.width = `${(pos / st.duration) * 100}%`;
    $('pos').textContent = mmss(pos);
    $('dur').textContent = mmss(st.duration);
  };

  const render = async () => {
    try {
      st = await invoke('media_state');
    } catch {
      st = null;
    }
    const on = st && st.active;
    card.classList.toggle('idle', !on);
    $('src').textContent = on ? st.source : '';
    $('title').textContent = on ? st.title || 'Без названия' : 'Ничего не играет';
    $('title').title = on ? st.title : '';
    $('artist').textContent = on ? st.artist || st.album : 'Включите музыку в любом плеере';
    $('toggle').innerHTML = on && st.playing ? ICON.pause : ICON.play;
    $('prev').disabled = !on || !st.canPrev;
    $('next').disabled = !on || !st.canNext;
    $('toggle').disabled = !on;

    const key = on ? st.artKey : '';
    if (key !== artKey) {
      artKey = key;
      const art = key ? await invoke('media_art').catch(() => null) : null;
      if (artKey === key) $('art').innerHTML = art ? `<img src="${art}" alt="">` : ICON.note;
    }
    paintProgress();
  };

  const control = async (action) => {
    if (action === 'toggle' && st) {
      st.playing = !st.playing; // instant feedback; the next poll confirms
      $('toggle').innerHTML = st.playing ? ICON.pause : ICON.play;
    }
    await invoke('media_control', { action }).catch(() => {});
    setTimeout(render, 250);
  };
  ['prev', 'toggle', 'next'].forEach((a) => ($(a).onclick = () => control(a)));

  render();
  setInterval(render, 1500);
  setInterval(paintProgress, 500);
}

// ---------------- weather: primary city + other cities ----------------
const ARROW = '<svg viewBox="0 0 24 24"><path d="M21 3 3 10.5l7.5 3L13.5 21z" fill="currentColor"/></svg>';

function weather() {
  card.classList.add('auto');
  let list = [];
  const data = new Map(); // "lat,lon" -> forecast
  const key = (c) => `${c.lat},${c.lon}`;

  const primaryHtml = (city, d) => {
    if (!city) return '<div class="msg">Добавьте город через меню «Города…»</div>';
    if (!d) return `<div class="top"><div><div class="city">${city.name} ${ARROW}</div></div></div><div class="msg">Загрузка…</div><div></div>`;
    const cur = d.current;
    const now = cur.time.slice(0, 13);
    const start = Math.max(0, d.hourly.time.findIndex((t) => t.slice(0, 13) === now));
    let hours = '';
    for (let i = start; i < start + 6 && i < d.hourly.time.length; i++) {
      const label = i === start ? 'Сейчас' : d.hourly.time[i].slice(11, 13);
      hours += `<div><div class="t">${label}</div>${wxIcon(wxKind(d.hourly.weather_code[i], d.hourly.is_day[i]))}<div class="v">${deg(d.hourly.temperature_2m[i])}</div></div>`;
    }
    return `
      <div class="top">
        <div>
          <div class="city">${city.name} ${ARROW}</div>
          <div class="temp">${deg(cur.temperature_2m)}</div>
        </div>
        <div class="right">
          ${wxIcon(wxKind(cur.weather_code, cur.is_day))}
          <div class="cond">${WX_TEXT[cur.weather_code] ?? ''}</div>
          <div class="hilo">Макс.: ${deg(d.daily.temperature_2m_max[0])} Мин.: ${deg(d.daily.temperature_2m_min[0])}</div>
        </div>
      </div>
      <div class="hours">${hours}</div>`;
  };

  const rowHtml = (c, i) => {
    const d = data.get(key(c));
    if (!d) {
      return `<div class="row" data-nodrag data-i="${i}" title="Сделать основным"><div class="info"><div class="name">${c.name}</div><div class="sub">Загрузка…</div></div></div>`;
    }
    const cur = d.current;
    return `
      <div class="row" data-nodrag data-i="${i}" title="Сделать основным">
        <div class="info">
          <div class="name">${c.name}</div>
          <div class="sub">${cityTime(d)} · ${WX_TEXT[cur.weather_code] ?? ''}</div>
        </div>
        ${wxIcon(wxKind(cur.weather_code, cur.is_day))}
        <div class="tv">
          <div class="t">${deg(cur.temperature_2m)}</div>
          <div class="hl">↑${deg(d.daily.temperature_2m_max[0])} ↓${deg(d.daily.temperature_2m_min[0])}</div>
        </div>
      </div>`;
  };

  const render = () => {
    const [primary, ...others] = list;
    const pd = primary && data.get(key(primary));
    card.className = 'card auto';
    card.innerHTML = `
      <div class="wxall">
        <div class="wx">${primaryHtml(primary, pd)}</div>
        ${others.length ? `<div class="others">${others.map((c, i) => rowHtml(c, i + 1)).join('')}</div>` : ''}
        <div class="foot"><span class="edit" data-nodrag id="edit">Изменить список городов…</span></div>
      </div>`;
    document.getElementById('edit').onclick = () => invoke('open_cities');
    card.querySelectorAll('.row[data-i]').forEach((row) => {
      row.onclick = () => makePrimary(+row.dataset.i);
    });
    observeFit();
  };

  // Clicking a city in the list makes it the main one (moves it to the top).
  const makePrimary = (i) => {
    const next = [list[i], ...list.filter((_, j) => j !== i)];
    list = next;
    render();
    invoke('set_cities', { cities: next });
  };

  const load = async () => {
    list = (await invoke('get_settings')).cities;
    render();
    await Promise.all(list.map(async (c) => {
      try { data.set(key(c), await fetchWeather(c)); } catch { /* keep previous */ }
    }));
    render();
  };
  load();
  setInterval(load, WEATHER_EVERY);
  setInterval(render, 30000); // keep local times fresh
  listen('settings-changed', load);
  listen('refresh', load);
}

({ clock, calendar, system: systemWidget, weather, media })[kind]?.();
