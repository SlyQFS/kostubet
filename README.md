# Kostubet GitHub Watcher Telegram Bot 🚀

Высокопроизводительный легкий бота на Rust, который отслеживает репозитории GitHub (релизы, теги, коммиты) и отправляет оформленные Rich Text карточки в топик супергруппы Telegram.

---

## ✨ Особенности

- **Управление в ЛС (без мусора в группе)**: Все команды управления (`/admin`, `/test`, `/track`, `/untrack`, `/list`, `/help`) выполняются **исключительно в ЛС с ботом**.
- **Ограничение прав**: Команды управления доступны **только Администраторам группы** или пользователям из `ADMIN_USER_IDS`.
- **Гибкие переключатели**: Возможность включать/отключать проверку Релизов, Тегов и Коммитов (`CHECK_RELEASES`, `CHECK_TAGS`, `CHECK_COMMITS`).
- **Ультра-легкий контейнер**: Сборка на базе Alpine Linux (~20 МБ размер итогового образа).
- **Красивые Rich Text карточки**: Поддержка заголовков, жирного шрифта, курсива, блоков кода и Telegram `<blockquote expandable>` с автоотключением внешних баннеров (OG link preview).
- **Нулевой расход лимитов (ETag Caching)**: Использование HTTP-заголовков `If-None-Match`. Неизменившиеся репозитории возвращают `304 Not Modified` и **не тратят лимит запросов GitHub**.
- **Надежность SQLite**: Информация об отправленных релизах сохраняется в SQLite `kostubet.db` **только после подтверждения доставки** в Telegram.

---

## 🚀 Запуск в Docker

### Вариант 1. Запуск через Docker Compose (Рекомендуемый)

1. Создайте файл `.env` из примера:
   ```bash
   cp .env.example .env
   ```

2. Заполните `.env`:
   ```env
   TELEGRAM_BOT_TOKEN=7123456789:ABCdefGHIjklMNOpqrsTUVwxyZ
   TELEGRAM_CHAT_ID=-1001987654321
   TELEGRAM_ARCHIVE_THREAD_ID=42
   ADMIN_USER_IDS=123456789
   TRACKED_REPOS=tokio-rs/tokio,rust-lang/rust

   # Переключатели проверок (true / false)
   CHECK_RELEASES=true
   CHECK_TAGS=true
   CHECK_COMMITS=false

   POLL_INTERVAL_SECS=900
   ```

3. Запустите одной командой:
   ```bash
   docker compose up -d
   ```

---

### Вариант 2. Прямая команда `docker run` (со всеми флагами `-e`)

1. Соберите образ:
   ```bash
   docker build -t kostubet-github .
   ```

2. Запустите контейнер со всеми флагами:
   ```bash
   docker run -d \
     --name kostubet-github \
     -e TELEGRAM_BOT_TOKEN="7123456789:ABCdefGHIjklMNOpqrsTUVwxyZ" \
     -e TELEGRAM_CHAT_ID="-1001987654321" \
     -e TELEGRAM_ARCHIVE_THREAD_ID="42" \
     -e ADMIN_USER_IDS="123456789" \
     -e TRACKED_REPOS="tokio-rs/tokio,rust-lang/rust" \
     -e CHECK_RELEASES="true" \
     -e CHECK_TAGS="true" \
     -e CHECK_COMMITS="false" \
     -e POLL_INTERVAL_SECS="900" \
     -v $(pwd)/data:/app/data \
     --restart unless-stopped \
     kostubet-github
   ```

---

## 🤖 Команды управления в ЛС (Direct Messages)

| Команда | Описание |
|---|---|
| `/admin` | Открыть панель администратора со статусом и настройками |
| `/test` | Отправить Rich Text тестовую карточку релиза в топик группы |
| `/track owner/repo` | Добавить репозиторий в список отслеживания |
| `/untrack owner/repo` | Удалить репозиторий |
| `/list` | Показать список отслеживаемых репозиториев и их версии |
| `/help` | Показать отладочную информацию (User ID, Chat ID, Thread ID) |

---

## 📄 Лицензия

MIT License.
