// Open-Meteo helpers (no API key) and SVG weather icons. Shared via window globals.

const WX_TEXT = {
  0: 'Ясно', 1: 'Преимущественно ясно', 2: 'Переменная облачность', 3: 'Пасмурно',
  45: 'Туман', 48: 'Изморозь',
  51: 'Слабая морось', 53: 'Морось', 55: 'Сильная морось', 56: 'Ледяная морось', 57: 'Ледяная морось',
  61: 'Небольшой дождь', 63: 'Дождь', 65: 'Сильный дождь', 66: 'Ледяной дождь', 67: 'Ледяной дождь',
  71: 'Небольшой снег', 73: 'Снег', 75: 'Сильный снег', 77: 'Снежная крупа',
  80: 'Ливень', 81: 'Ливень', 82: 'Сильный ливень', 85: 'Снегопад', 86: 'Сильный снегопад',
  95: 'Гроза', 96: 'Гроза с градом', 99: 'Гроза с градом',
};

function wxKind(code, isDay) {
  if (code <= 1) return isDay ? 'clear-day' : 'clear-night';
  if (code === 2) return isDay ? 'partly-day' : 'partly-night';
  if (code === 3) return 'cloudy';
  if (code === 45 || code === 48) return 'fog';
  if ([56, 57, 66, 67].includes(code)) return 'sleet';
  if (code >= 51 && code <= 55) return 'drizzle';
  if ((code >= 61 && code <= 65) || (code >= 80 && code <= 82)) return 'rain';
  if ((code >= 71 && code <= 77) || code === 85 || code === 86) return 'snow';
  if (code >= 95) return 'thunder';
  return 'cloudy';
}

const SUN = '#ffd60a';
const CLOUD = (fill) =>
  `<g fill="${fill}"><circle cx="34" cy="58" r="15"/><circle cx="53" cy="46" r="21"/>` +
  `<circle cx="71" cy="59" r="14"/><rect x="20" y="56" width="64" height="17" rx="8.5"/></g>`;
const sun = (r = 17) => {
  let rays = '';
  for (let i = 0; i < 8; i++) {
    const a = (i * Math.PI) / 4;
    const p = (d) => `${(50 + Math.cos(a) * d).toFixed(1)} ${(50 + Math.sin(a) * d).toFixed(1)}`;
    rays += `<path d="M${p(r + 9)}L${p(r + 17)}"/>`;
  }
  return `<g stroke="${SUN}" stroke-width="6" stroke-linecap="round">${rays}</g><circle cx="50" cy="50" r="${r}" fill="${SUN}"/>`;
};
const MOON = '<path d="M60 18a32 32 0 1 0 24 50A26 26 0 0 1 60 18z" fill="#fde9a6"/>';
const drops = (c) =>
  `<g stroke="${c}" stroke-width="5" stroke-linecap="round"><path d="M37 78l-4 11"/><path d="M53 78l-4 11"/><path d="M69 78l-4 11"/></g>`;
const flakes = '<g fill="#fff"><circle cx="35" cy="83" r="4"/><circle cx="52" cy="89" r="4"/><circle cx="69" cy="83" r="4"/></g>';

function wxIcon(kind, cls = 'ic') {
  const up = (s) => `<g transform="translate(0 -12)">${s}</g>`;
  const body = {
    'clear-day': sun(),
    'clear-night': MOON,
    'partly-day': `<g transform="translate(62 32) scale(.62) translate(-50 -50)">${sun()}</g><g transform="translate(-4 8)">${CLOUD('#fff')}</g>`,
    'partly-night': `<g transform="translate(64 30) scale(.55) translate(-50 -50)">${MOON}</g><g transform="translate(-4 8)">${CLOUD('#e9edf3')}</g>`,
    'cloudy': `<g transform="translate(0 2)">${CLOUD('#eef1f5')}</g>`,
    'fog': up(CLOUD('#e3e7ed')) + '<g stroke="#e3e7ed" stroke-width="5" stroke-linecap="round" opacity=".8"><path d="M24 78h52"/><path d="M32 89h40"/></g>',
    'drizzle': up(CLOUD('#eef1f5')) + drops('#7fd4ff'),
    'rain': up(CLOUD('#dde2e9')) + drops('#4fc3ff'),
    'snow': up(CLOUD('#eef1f5')) + flakes,
    'sleet': up(CLOUD('#dde2e9')) + '<g stroke="#4fc3ff" stroke-width="5" stroke-linecap="round"><path d="M37 78l-4 11"/><path d="M69 78l-4 11"/></g><circle cx="52" cy="86" r="4" fill="#fff"/>',
    'thunder': up(CLOUD('#d3d9e1')) + `<path d="M55 64 41 84h11l-6 15 18-22H52l6-13z" fill="${SUN}"/>`,
  }[kind] || CLOUD('#fff');
  return `<svg class="${cls}" viewBox="0 0 100 100">${body}</svg>`;
}

async function fetchWeather(c) {
  const url = 'https://api.open-meteo.com/v1/forecast'
    + `?latitude=${c.lat}&longitude=${c.lon}`
    + '&current=temperature_2m,apparent_temperature,weather_code,is_day'
    + '&hourly=temperature_2m,weather_code,is_day'
    + '&daily=temperature_2m_max,temperature_2m_min'
    + '&timezone=auto&forecast_days=2';
  const r = await fetch(url);
  if (!r.ok) throw new Error(`HTTP ${r.status}`);
  return r.json();
}

/** Current wall-clock time in the city, from Open-Meteo's UTC offset. */
function cityTime(data) {
  const d = new Date(Date.now() + data.utc_offset_seconds * 1000);
  return `${String(d.getUTCHours()).padStart(2, '0')}:${String(d.getUTCMinutes()).padStart(2, '0')}`;
}

const deg = (v) => `${Math.round(v)}°`;
