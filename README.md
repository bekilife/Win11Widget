<div align="center">

<img src="assets/logo.png" width="96" alt="Win11Widget">

# Win11Widget

**Виджеты рабочего стола для Windows 11 в стиле macOS**

Курсы · Сейчас играет · Погода · Система

[![Build](../../actions/workflows/build.yml/badge.svg)](../../actions/workflows/build.yml)
![Platform](https://img.shields.io/badge/platform-Windows%2010%20%7C%2011-0078D4?logo=windows11&logoColor=white)
![Win32](https://img.shields.io/badge/Win32-native-0078D4)
![Rust](https://img.shields.io/badge/Rust-stable-000000?logo=rust)
![License](https://img.shields.io/badge/license-MIT-green)

[**Скачать**](../../releases/latest) · [Возможности](#возможности) · [Сборка из исходников](#сборка-из-исходников) · [English](#english)

<img src="docs/screenshot.png" width="360" alt="Скриншот виджетов">

</div>

## Возможности

| Виджет | Что показывает |
|---|---|
| **Курсы** | Доллар к тенге (цена продажи в обменнике mig.kz), биткойн и Gram к доллару с Binance — с изменением и мини-графиком за сутки. Обновляется раз в минуту |
| **Сейчас играет** | Обложка, трек, исполнитель, прогресс и кнопки ⏮ ⏯ ⏭ для любого плеера: Spotify, Яндекс Музыка, YouTube в браузере, VLC и др. |
| **Погода** | Основной город с прогнозом на 6 часов и список других городов с местным временем. Клик по городу делает его основным |
| **Система** | Компактно: кольца процессора, памяти и видеокарты. При наведении раскрывается полная версия: модель CPU, ОЗУ, GPU с температурой и VRAM, сеть, время работы, все диски |

Кроме того:

- **Эффект стекла** — под каждым виджетом размытый фрагмент ваших обоев, скругление 22 px.
- **Живут на рабочем столе** — не перекрывают окна, не пропадают по Win+D, нет в панели задач и Alt+Tab.
- **Сетка** — одинаковые отступы 16 px; при перетаскивании виджет примагничивается к соседям и краям экрана; если виджет меняет высоту, нижние сдвигаются следом.
- **Иконка в трее** — включение и выключение виджетов, список городов, автозапуск, «Расставить заново».
- **Лёгкий** — один процесс и один exe около 600 КБ, ~13 МБ памяти; без WebView2, фоновых служб и сторонних драйверов.

## Установка

1. Скачайте `win11widget.exe` со страницы [Releases](../../releases/latest).
2. Положите в любую папку, например `%LOCALAPPDATA%\Win11Widget`, и запустите. Установка и права администратора не нужны.
3. Виджеты появятся у правого края экрана, иконка — в трее. Автозапуск включается в меню трея.

> Файл не подписан цифровой подписью, поэтому SmartScreen может показать предупреждение: «Подробнее» → «Выполнить в любом случае».

Требуется Windows 10 1809+ или Windows 11.

### Переход с версии 1.x (Tauri)

Настройки (`%APPDATA%\com.win11widget.desktop\settings.json`) и запись автозапуска `Win11Widget` те же — города и позиции виджетов сохранятся. Закройте старую версию (трей → «Выход») и замените `%LOCALAPPDATA%\Win11Widget\win11widget.exe` новым файлом.

## Использование

- **Перетаскивание** — за любое место виджета; позиция запоминается.
- **Правый клик по виджету** — скрыть виджет, «Города…», «Обновить», «Расставить заново», «Выход».
- **Города** — поиск на любом языке, ★ отмечает основной город, порядок меняется стрелками.
- **Автозапуск** — меню трея → «Запускать вместе с Windows».

## Как это устроено

| Задача | Решение |
|---|---|
| Окна виджетов | Слоистые окна Win32 (`WS_EX_LAYERED`) с попиксельной прозрачностью, владелец — окно рабочего стола (`Progman`) |
| Отрисовка | Direct2D (программный растеризатор, без видеоустройства) и DirectWrite, шрифт Segoe UI Variable |
| Курсы | Страница [mig.kz](https://mig.kz) (USD) и публичный API Binance (`BTCUSDT`, `GRAMUSDT`) через WinHTTP |
| Погода и поиск городов | [Open-Meteo](https://open-meteo.com) через WinHTTP — бесплатно, без ключа |
| Сейчас играет | Windows `GlobalSystemMediaTransportControls` (тот же источник, что у системной панели громкости) |
| CPU, память, сеть, диски | Win32 API напрямую |
| Видеокарта | D3DKMT (как в «Диспетчере задач»): загрузка, VRAM, температура, мощность (% от лимита) и вентилятор — для любого производителя |
| «Стекло» | Обои читаются через WIC, уменьшаются в 4 раза, размываются один раз; виджет рисует кусок под собой |

```
src/
  main.rs       окна, трей, меню, таймеры, перетаскивание
  widgets.rs    внешний вид каждого виджета
  cities.rs     окно редактирования городов
  gfx.rs        Direct2D/DirectWrite, слоистые окна
  icons.rs      векторные иконки (погода, плеер, система)
  layout.rs     сетка, примагничивание, раскрытие «Системы»
  system.rs     CPU, RAM, сеть, диски
  gpu.rs        видеокарта через D3DKMT
  media.rs      «Сейчас играет»
  weather.rs    Open-Meteo
  rates.rs      курсы: mig.kz и Binance
  wallpaper.rs  обои и размытие
  settings.rs   settings.json (формат версии 1.x)
  platform.rs   рабочий стол, мониторы, автозапуск
```

Настройки хранятся в `%APPDATA%\com.win11widget.desktop\settings.json`.

## Конфиденциальность

Приложение ничего не собирает и не отправляет. Сетевые запросы — только погода и поиск городов в Open-Meteo (в запросе только координаты выбранных городов или поисковая строка) и курсы: главная страница mig.kz и публичный API Binance. Телеметрии нет.

## Ограничения

- Только Windows.
- Встроенные видеокарты (без своей видеопамяти) не показываются — тогда вместо третьего кольца показывается диск C.
- Температура процессора не показывается: в Windows она доступна только через драйвер с правами администратора.

## Сборка из исходников

Нужны [Rust](https://rustup.rs) (stable) и [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) с компонентом «Разработка классических приложений на C++».

```bash
git clone https://github.com/bekilife/Win11Widget.git
cd Win11Widget
cargo build --release    # target/release/win11widget.exe
```

Проверки, которые выполняет CI:

```bash
cargo fmt --check
cargo clippy --release -- -D warnings
```

## Лицензия

[MIT](LICENSE)

---

## English

**Win11Widget** brings macOS-style desktop widgets to Windows 11: exchange rates (USD/KZT from mig.kz, BTC and GRAM from Binance), now playing, weather (Open-Meteo, no API key) and a system monitor (CPU, RAM, GPU with temperature, network, all drives). Widgets live on the desktop layer with a frosted-glass look, snap to an even 16 px grid and are managed from the tray. Native Win32 in Rust (Direct2D + DirectWrite, no WebView): a single ~600 KB executable using about 13 MB of RAM.

Download `win11widget.exe` from [Releases](../../releases/latest), or build it with `cargo build --release`. The UI is in Russian.
