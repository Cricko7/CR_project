# Multi-Agent Simulation Backend (Rust)

Документ актуален на: **2026-02-16**  
Этот README полностью отражает текущее состояние кода в репозитории `CR_project`.

## 1. Что это за проект

Backend для симуляции мира автономных AI-агентов:
- у агентов есть личность, состояние настроения, память;
- агенты выполняют "тики жизни" и порождают события;
- память хранится в PostgreSQL + векторно индексируется в Qdrant;
- для генерации действий/суммаризации используется **Google Gemini API** (с fallback-режимом без LLM);
- есть HTTP API + WebSocket-стрим событий.

Проект сейчас сфокусирован на backend-ядре. Полноценный web-frontend в этом репозитории **не реализован**.

---

## 2. Быстрый статус по требованиям хакатона

| Требование | Статус | Что есть сейчас |
|---|---|---|
| 1. Долговременная память (vector DB + summarization) | `FULLY IMPLEMENTED` | Реализованы `memory_entries`, embedding pipeline с retry+DLQ, Qdrant recall, архивирование старых воспоминаний, авто-суммаризация overflow, API для dead-letter requeue |
| 2. Эмоциональный интеллект | `PARTIAL` | Поля valence/arousal/mood есть, но динамика эмоций и mood decay пока не реализованы (воркер-заглушка) |
| 3. Архитектура агента (рефлексия→цель→действие) | `PARTIAL` | Есть tick orchestrator + LLM summary, но отдельные стадии планирования пока не декомпозированы |
| 4. Мультиагентность (общение, отношения) | `PARTIAL` | Таблицы `messages`, `relationships` есть; процессинг межагентных сообщений/обновления отношений пока не поднят |
| 5. Real-time dashboard life feed | `PARTIAL` | Есть `/events` + `/ws/events`; живой WS-пуш пока только для тиков, инициированных через API |
| 6. Граф отношений | `NOT STARTED` | Модель таблицы есть, API/агрегации/стримов отношений нет |
| 7. Inspector агента | `PARTIAL` | Есть `/agents/{id}/state`, memory recall; агрегированного "профиля агента" endpoint пока нет |
| 8. Панель вмешательства | `PARTIAL` | Есть endpoint ручного тика и memory append; отдельные intervention endpoints пока не готовы |
| Доп. фича 1: настроение влияет на стиль речи | `PARTIAL` | Mood передается в prompt, но полноценные style policies не оформлены |
| Доп. фича 2: страница агента с историей отношений | `NOT STARTED` | Нужен frontend + API для отношений и timeline |

---

## 3. Архитектура (текущее состояние)

### 3.1 Контекст (C4 Level 1)

```mermaid
flowchart LR
    Admin[Admin/User UI] -->|HTTP + WebSocket| API[API Service - Axum]
    API -->|SQL| PG[(PostgreSQL)]
    API -->|Vector Search/Upsert| QD[(Qdrant)]
    API -->|LLM + Embeddings| GEM[Google Gemini API]

    Worker[Background Worker] -->|SQL| PG
    Worker -->|Vector Upsert/Search| QD
    Worker -->|LLM + Embeddings| GEM
```

### 3.2 Контейнеры (C4 Level 2)

```mermaid
flowchart TB
    subgraph RustWorkspace[Cargo Workspace]
        APIBin[crates/api\nHTTP + WS boundary]
        WorkerBin[crates/worker\nbackground jobs]
        CoreLib[crates/sim-backend\ndomain + ports + adapters]
    end

    APIBin --> CoreLib
    WorkerBin --> CoreLib
    CoreLib --> PG[(PostgreSQL)]
    CoreLib --> QD[(Qdrant)]
    CoreLib --> GEM[Gemini API]
```

### 3.3 Компоненты AgentCore (C4 Level 3)

```mermaid
flowchart LR
    TickReq[Tick request]
    Orchestrator[AgentTickOrchestrator]
    Runner[AgentTickRunner\nsingle-flight per agent]
    RepoPort[AgentCoreRepository trait]
    LlmPort[LlmPort trait]
    PgRepo[PostgresAgentCoreRepository]
    Gemini[GeminiClient]
    Events[(events table)]
    States[(agent_states table)]
    Agents[(agents table)]

    TickReq --> Orchestrator
    Orchestrator --> Runner
    Runner --> Orchestrator
    Orchestrator --> RepoPort
    Orchestrator --> LlmPort
    RepoPort --> PgRepo
    LlmPort --> Gemini
    PgRepo --> Agents
    PgRepo --> States
    PgRepo --> Events
```

---

## 4. Bounded Contexts

### AgentCore
- Файлы: `crates/sim-backend/src/agent_core/*`
- Ответственность:
  - выполнение agent tick;
  - dedup/busy protection на процесс;
  - запись `agent_states` и `events`;
  - LLM-driven action summary с fallback.

### Memory
- Файлы: `crates/sim-backend/src/memory/*`
- Ответственность:
  - append episodic memory;
  - embedding pipeline (pending -> embedded/failed);
  - recall по векторному поиску;
  - overflow summarization и архивирование старых записей.

### Communication
- Текущее состояние:
  - есть `messages` таблица, но нет полноценного message bus/consumer flow.

### Emotions
- Текущее состояние:
  - хранятся `valence/arousal/mood_label`, но evolution logic почти отсутствует.

### Interventions
- Текущее состояние:
  - есть `interventions` таблица и частично ручные API-операции;
  - нет завершенного workflow вмешательств.

### Observability
- Файлы: `crates/sim-backend/src/app/observability.rs`, `runtime.rs`
- Ответственность:
  - JSON structured logs через `tracing`;
  - graceful shutdown через cancellation token + timeout.
- Gap:
  - Prometheus metrics пока не реализованы.

---

## 5. Техстек и ключевые решения

| Слой | Выбор |
|---|---|
| Язык | Rust (edition 2024) |
| HTTP/WS | Axum |
| Runtime | Tokio |
| DB | PostgreSQL (`sqlx`) |
| Vector DB | Qdrant (HTTP API) |
| LLM | Google Gemini API |
| Embeddings | Gemini embeddings (`text-embedding-004` по умолчанию) или локальный hash fallback |
| Логи | tracing + tracing-subscriber (JSON) |
| Graceful shutdown | custom `ServiceRuntime` + cancellation token |

Почему Rust здесь подходит:
- безопасная конкуррентность для многопоточного worker runtime;
- predictable performance для frequent ticks/embedding jobs;
- строгие типы и trait ports помогают держать clean boundaries.

---

## 6. Структура репозитория

```text
CR_project/
├── Cargo.toml                      # workspace
├── crates/
│   ├── api/
│   │   └── src/main.rs             # HTTP + WS service
│   ├── worker/
│   │   └── src/main.rs             # background workers
│   └── sim-backend/
│       ├── migrations/
│       │   ├── 0001_initial_schema.sql
│       │   ├── 0002_memory_long_term.sql
│       │   ├── 0003_concurrency_failure_modes.sql
│       │   └── 0004_memory_embedding_retry_dlq.sql
│       └── src/
│           ├── app/                # config, runtime, observability
│           ├── agent_core/         # orchestrator, persistence traits, tick runner
│           ├── memory/             # memory service + ports
│           ├── infrastructure/     # postgres, qdrant, gemini adapters
│           ├── llm/                # LLM port
│           └── lib.rs
└── README.md
```

---

## 7. Runtime потоки (как система работает сейчас)

### 7.1 Tick flow
1. API (`POST /agents/{id}/ticks`) или worker tick loop вызывает `AgentTickOrchestrator`.
2. Orchestrator сначала проверяет persisted idempotency и берет глобальный lease на агента в Postgres:
   - completed tick id -> `SkippedDuplicate`;
   - lease занят другим процессом -> `SkippedBusy`.
3. `AgentTickRunner` защищает от:
   - duplicate tick id;
   - concurrent tick для одного агента в пределах одного процесса.
4. Orchestrator:
   - читает agent + state;
   - upsert state;
   - генерирует action summary (Gemini или fallback);
   - пишет event в `events`.
5. После завершения (success/error) tick id фиксируется в persisted history, lease освобождается.
6. После успешного тика сервис пытается append memory с текстом результата.

### 7.2 Memory embedding flow
1. Новая память попадает в `memory_entries` со `embedding_status='pending'`.
2. Worker job `memory_embedding_worker` (или API endpoint вручную) атомарно claim'ит due batch через `FOR UPDATE SKIP LOCKED` и переводит записи в `embedding_status='processing'`.
3. Embedding через GeminiEmbeddingClient или локальный hash embedder.
4. Vector upsert в Qdrant.
5. Статус в Postgres:
   - success -> `embedded` + `embedding_model`;
   - transient failure -> `pending` + `embedding_error` + retry backoff;
   - max attempts exceeded -> `dead_letter` + `embedding_dead_lettered_at`.
6. Если воркер упал на середине, `processing` записи можно reclaim'ить после claim-timeout.
7. Dead-letter записи доступны через API и могут быть requeue вручную.

### 7.3 Memory overflow summarization flow
1. Проверяется count активных memories агента.
2. Если > `max_active`, выбираются самые старые записи.
3. Формируется summary (LLM, иначе deterministic fallback).
4. Summary вставляется как `is_summary=true`.
5. Источники архивируются (`archived=true`, `summarized_by_id`).

### 7.4 WebSocket flow
1. Клиент подключается к `/ws/events`.
2. Получает snapshot последних events.
3. Получает live-сообщения из in-memory `broadcast` hub.
4. При лаге клиента соединение закрывается с ошибкой stream lagged.

Ограничение:
- live WS сейчас публикуется только для тиков, инициированных в API процессе.
- события, созданные worker-ом, в тот же момент через WS не пушатся (видны через snapshot/polling).

---

## 8. API Reference (текущее)

Базовый URL по умолчанию: `http://127.0.0.1:8080`

### 8.1 Health

#### `GET /health`
#### `GET /livez`

Response:
```json
{
  "status": "ok",
  "service": "sim-backend"
}
```

### 8.2 Agent Tick

#### `POST /agents/{id}/ticks`

Request (optional):
```json
{
  "tick_id": "custom-idempotency-key"
}
```

Success (`200`):
```json
{
  "outcome": "applied",
  "agent_id": "uuid",
  "tick_id": "tick-id",
  "event_id": 123,
  "mood_label": "neutral",
  "valence": 0.0,
  "arousal": 0.0
}
```

Conflict (`409`) when busy/duplicate:
```json
{
  "outcome": "skipped_busy",
  "agent_id": "uuid",
  "tick_id": "tick-id",
  "event_id": null,
  "mood_label": null,
  "valence": null,
  "arousal": null
}
```

Not found (`404`):
```json
{
  "error": "agent_not_found",
  "message": "agent `<id>` does not exist"
}
```

### 8.3 Agent State

#### `GET /agents/{id}/state`

Response (`200`):
```json
{
  "agent_id": "uuid",
  "mood_label": "neutral",
  "valence": 0.0,
  "arousal": 0.0,
  "updated_at": "2026-02-16T12:00:00Z"
}
```

### 8.4 Events feed

#### `GET /events?agent_id=<uuid>&limit=<1..200>`

Response:
```json
{
  "items": [
    {
      "id": 1,
      "agent_id": "uuid",
      "event_type": "agent.tick.executed",
      "description": "Agent `Alice` executed tick ...",
      "payload": "{\"tick_id\":\"...\"}",
      "occurred_at": "2026-02-16T12:00:00Z"
    }
  ]
}
```

### 8.5 Memory append

#### `POST /agents/{id}/memories`

Request:
```json
{
  "content": "Agent found a treasure map",
  "importance": 0.8
}
```

Response (`201`):
```json
{
  "memory_id": 42,
  "embedding_status": "pending"
}
```

### 8.6 Memory recall

#### `GET /agents/{id}/memories/recall?query=...&top_k=8`

Response:
```json
{
  "items": [
    {
      "memory_id": 42,
      "score": 0.91,
      "content": "Agent found a treasure map",
      "summary": null,
      "importance": 0.8,
      "created_at": "2026-02-16T12:00:00Z"
    }
  ]
}
```

### 8.7 Manual summarization

#### `POST /agents/{id}/memories/summarize`

Request (optional):
```json
{
  "max_active": 200,
  "batch_size": 20
}
```

Response:
```json
{
  "created_summary": true,
  "source_count": 20,
  "summary_entry_id": 1001
}
```

### 8.8 Manual embedding processing

#### `POST /memory/process-embeddings`

Request (optional):
```json
{
  "limit": 50
}
```

Response:
```json
{
  "processed": 10,
  "succeeded": 8,
  "failed": 2,
  "retried": 1,
  "dead_lettered": 1
}
```

### 8.9 Dead-letter embeddings

#### `GET /memory/dead-letter?limit=<1..200>`

Response:
```json
{
  "items": [
    {
      "memory_id": 42,
      "agent_id": "uuid",
      "content": "Agent found a treasure map",
      "summary": null,
      "importance": 0.8,
      "created_at": "2026-02-16T12:00:00Z",
      "embedding_status": "dead_letter"
    }
  ]
}
```

#### `POST /memory/dead-letter/{memory_id}/requeue`

Response:
```json
{
  "memory_id": 42,
  "requeued": true
}
```

### 8.10 WebSocket events

#### `GET /ws/events?agent_id=<uuid>&snapshot_limit=<1..200>`

Server event envelope (`snake_case`, `type` discriminator):
- `snapshot`
- `tick_applied`
- `tick_skipped`
- `error`

Examples:
```json
{"type":"snapshot","items":[...]}
```

```json
{
  "type":"tick_applied",
  "agent_id":"uuid",
  "tick_id":"...",
  "event_id":123,
  "mood_label":"neutral",
  "valence":0.0,
  "arousal":0.0
}
```

---

## 9. База данных и векторное хранилище

### 9.1 PostgreSQL schema

Миграции:
- `crates/sim-backend/migrations/0001_initial_schema.sql`
- `crates/sim-backend/migrations/0002_memory_long_term.sql`
- `crates/sim-backend/migrations/0003_concurrency_failure_modes.sql`
- `crates/sim-backend/migrations/0004_memory_embedding_retry_dlq.sql`

Основные таблицы:
- `agents`: карточка агента, personality JSON.
- `agent_states`: valence/arousal/mood.
- `events`: доменные события.
- `memory_entries`: эпизодическая и summary память.
- `relationships`: каркас для affinity.
- `messages`: каркас межагентных/пользовательских сообщений.
- `interventions`: каркас админ-вмешательств.
- `outbox_events`: каркас event publication.

Колонки long-term memory:
- `is_summary`
- `archived`
- `summarized_by_id`
- `embedding_model`
- `embedding_error`
- `embedding_attempts`
- `embedding_next_retry_at`
- `embedding_dead_lettered_at`
- `last_accessed_at` (сейчас не обновляется)

### 9.2 Qdrant

Collection по умолчанию: `agent_memories`, vector size: `768`, distance: `Cosine`.

Payload в point:
- `memory_id`
- `agent_id`
- `importance`
- `is_summary`
- `created_at`

---

## 10. Конфигурация окружения (env)

### 10.1 Общие

| Variable | Default | Назначение |
|---|---|---|
| `SERVICE_NAME` | `sim-backend` (api) / `sim-worker` (worker) | Имя сервиса в логах |
| `LOG_LEVEL` | `info` | Уровень логов |
| `SHUTDOWN_TIMEOUT_MS` | `15000` | Таймаут graceful shutdown |

### 10.2 API network

| Variable | Default |
|---|---|
| `API_HOST` | `0.0.0.0` |
| `API_PORT` | `8080` |

### 10.3 Database

| Variable | Default |
|---|---|
| `DATABASE_URL` | required |
| `DATABASE_MAX_CONNECTIONS` | `10` |
| `DATABASE_CONNECT_TIMEOUT_MS` | `5000` |
| `DATABASE_ACQUIRE_TIMEOUT_MS` | `5000` |
| `DATABASE_IDLE_TIMEOUT_MS` | `60000` |
| `DATABASE_MAX_LIFETIME_MS` | `300000` |
| `DATABASE_RUN_MIGRATIONS` | `false` |

### 10.4 Gemini

| Variable | Default |
|---|---|
| `GEMINI_API_KEY` | empty -> LLM disabled |
| `GEMINI_MODEL` | `gemini-2.0-flash` |
| `GEMINI_EMBED_MODEL` | `text-embedding-004` |
| `GEMINI_BASE_URL` | `https://generativelanguage.googleapis.com` |
| `GEMINI_TIMEOUT_MS` | `15000` |
| `GEMINI_MAX_RETRIES` | `2` |
| `GEMINI_RETRY_BACKOFF_MS` | `300` |

### 10.5 Qdrant

| Variable | Default |
|---|---|
| `QDRANT_URL` | `http://localhost:6333` |
| `QDRANT_API_KEY` | optional |
| `QDRANT_COLLECTION` | `agent_memories` |
| `QDRANT_VECTOR_SIZE` | `768` |
| `QDRANT_TIMEOUT_MS` | `5000` |

### 10.6 Memory workers

| Variable | Default |
|---|---|
| `MEMORY_EMBED_BATCH_SIZE` | `32` |
| `MEMORY_MAX_ACTIVE_PER_AGENT` | `200` |
| `MEMORY_SUMMARY_BATCH_SIZE` | `20` |
| `MEMORY_EMBED_INTERVAL_MS` | `5000` |
| `MEMORY_SUMMARY_INTERVAL_MS` | `30000` |

### 10.7 Worker

| Variable | Default |
|---|---|
| `WORKER_AGENT_IDS` | empty |
| `WORKER_TICK_INTERVAL_MS` | `1000` |
| `WORKER_TICK_CONCURRENCY` | `8` |
| `WORKER_MOOD_DECAY_INTERVAL_MS` | `5000` |

---

## 11. Локальный запуск (с нуля)

### 11.1 Prerequisites
- Rust stable toolchain;
- Docker (для Postgres + Qdrant);
- доступ к Google Gemini API key (опционально, но нужен для LLM режима).

### 11.2 Поднять инфраструктуру

PostgreSQL:
```bash
docker run --name sim-pg -e POSTGRES_PASSWORD=postgres -e POSTGRES_DB=sim -p 5432:5432 -d postgres:16
```

Qdrant:
```bash
docker run --name sim-qdrant -p 6333:6333 -p 6334:6334 -d qdrant/qdrant:v1.12.5
```

### 11.3 Настроить env (пример)

```bash
set DATABASE_URL=postgres://postgres:postgres@localhost:5432/sim
set DATABASE_RUN_MIGRATIONS=true
set API_HOST=127.0.0.1
set API_PORT=8080
set WORKER_AGENT_IDS=<uuid1>,<uuid2>
set WORKER_TICK_CONCURRENCY=8
set GEMINI_API_KEY=<your_google_api_key>
```

Если `GEMINI_API_KEY` не задан, система работает в fallback-режиме:
- tick summaries deterministic;
- embeddings через local hash embedder.

### 11.4 Запуск сервисов

API:
```bash
cargo run -p api
```

Worker:
```bash
cargo run -p worker
```

### 11.5 Базовая проверка

```bash
curl http://127.0.0.1:8080/health
```

### 11.6 Минимальный seed агентов (SQL пример)

```sql
INSERT INTO agents (id, name, personality_json)
VALUES
('11111111-1111-1111-1111-111111111111', 'Alice', '{"traits":["curious","friendly"]}'),
('22222222-2222-2222-2222-222222222222', 'Bob', '{"traits":["competitive","sarcastic"]}');
```

---

## 12. Наблюдаемость и эксплуатация

Что уже есть:
- структурированные JSON логи (`tracing`);
- явные event/error логи для tick/memory воркеров;
- graceful shutdown с ожиданием фоновых задач.

Что отсутствует:
- Prometheus metrics endpoint;
- distributed tracing (trace-id across services);
- SLO/error budget dashboards.

---

## 13. Security review (текущее состояние)

### Что хорошо
- SQL запросы параметризованы через `sqlx` (снижение риска SQLi).
- Нет небезопасных `unsafe` участков.
- Конфиг/секреты читаются из env, не захардкожены в коде.
- Есть таймауты и retry для Gemini HTTP.

### Ключевые дыры/риски
1. **Нет аутентификации/авторизации** API и WS.
2. **Нет rate limiting** на дорогие endpoint'ы (`/ticks`, `/memories/*`, `/process-embeddings`).
3. **Нет tenant isolation** (single-world assumptions).
4. **In-memory WS hub** не защищен и не масштабируется межинстансно.
5. **Нет секрет-менеджмента** (Vault/KMS), только env vars.
6. **Нет audit trail** действий админа на уровне API policy.

### Минимум, который стоит сделать перед демо
1. Добавить простой JWT/API key guard на admin endpoints.
2. Добавить глобальный rate limit (IP + endpoint bucket).
3. Ограничить CORS и WS origins.

---

## 14. Concurrency и failure modes

### Уже учтено
- local single-flight для тиков одного агента (`Semaphore(1)` на agent id);
- duplicate tick id history per agent;
- persisted tick idempotency в Postgres (`agent_tick_dedup`);
- cross-process lease на tick per agent (`agent_tick_locks` с TTL);
- atomic claim для embedding jobs (`FOR UPDATE SKIP LOCKED`, `processing` + timeout reclaim);
- parallel tick processing в worker с ограничением `WORKER_TICK_CONCURRENCY`;
- graceful shutdown всех фоновых задач;
- worker loop не падает процессом при одиночных ошибках.

### Пропущенные failure modes
1. **Qdrant/Gemini partial outage**: нет circuit breaker / backpressure policy.
2. **WS live consistency**: worker-generated events не попадают в live hub API процесса.

---

## 15. ADR snapshot (фактически принятые решения)

| ADR | Решение | Почему | Цена |
|---|---|---|---|
| ADR-001 | Modular monolith в одном Rust workspace | Скорость хакатона + чистые границы | Ограниченная горизонтальная масштабируемость |
| ADR-002 | Отдельные бинарники `api` и `worker` | Разделение интерактивного и фонового workload | Требуется координация между процессами |
| ADR-003 | Postgres как source of truth | Простая транзакционность и SQL гибкость | Нужны миграции/индексы/операционная дисциплина |
| ADR-004 | Qdrant для долгой памяти | Быстрый vector recall с фильтром по агенту | Отдельный контур отказов |
| ADR-005 | Gemini как основной LLM+embeddings | Быстрый запуск без self-hosted моделей | Зависимость от внешнего API/квот |
| ADR-006 | Fallback режим без Gemini | Демо живет даже без API key | Качество summaries/recall ниже |
| ADR-007 | In-memory WS broadcast в API | Минимальная latency и простота | Не работает межинстансно, нет durable delivery |
| ADR-008 | Trait ports для repo/vector/llm | Тестируемость и сменяемость адаптеров | Больше кода и интерфейсных слоев |

---

## 16. Что уже сделано в коде

1. Базовый runtime и graceful shutdown.
2. Structured logging.
3. Конфигурирование через env с валидацией.
4. Agent tick orchestrator + LLM summary.
5. Постгрес-репозиторий для agent core.
6. Memory service:
   - append;
   - embedding pipeline с retry/backoff + DLQ;
   - vector recall;
   - overflow summarization.
7. Qdrant adapter и Gemini adapters.
8. REST endpoints для state/events/memory/ticks.
9. WebSocket endpoint с snapshot + live events.
10. Worker циклы: ticks, mood-decay (stub), embedding, summarization.
11. Миграции для базовой схемы, long-term memory и concurrency/failure hardening.
12. Unit tests для `agent_core` и `memory`.

---

## 17. Что нужно сделать, чтобы закрыть задачу "Создать веб-приложение..."

Приоритетный backlog:

### P0 (обязательно для демонстрации целевого кейса)
1. Реализовать auth (JWT/API key) для admin операций.
2. Добавить CRUD/API для:
   - relationships graph;
   - interventions (add event, send message);
   - agent inspector aggregate endpoint.
3. Поднять event bus для межагентных сообщений (Redis Streams/NATS) либо простой DB outbox + worker-consumer.
4. Сделать live updates для worker-событий в WS через общий pub/sub, а не process-local hub.
5. Реализовать эмоциональные переходы (из событий в mood update).

### P1 (нужно для полноты симуляции)
1. Разделить tick pipeline на этапы:
   - reflection;
   - goal selection;
   - action planning;
   - execution + side effects.
2. Реализовать relationship engine:
   - affinity update rules;
   - decay/recovery;
   - summary regeneration.
3. Реализовать message delivery lifecycle:
   - queued -> delivered -> acknowledged/failed.
4. Реализовать snapshot endpoints для frontend dashboard.

### P2 (hardening)
1. Prometheus metrics + `/metrics`.
2. Idempotency keys на уровне БД.
3. Circuit breaker policy для LLM/Qdrant outage.
4. Load testing и latency budgets.
5. CI pipeline с lint + test + migration check.

---

## 18. Гайд для другого ИИ (handoff section)

Если новый ИИ продолжает работу, начинай в таком порядке:

1. Прочитай entrypoints:
   - `crates/api/src/main.rs`
   - `crates/worker/src/main.rs`
   - `crates/sim-backend/src/lib.rs`
2. Пойми domain ports:
   - `agent_core/persistence.rs`
   - `memory/repository.rs`
   - `memory/vector_store.rs`
   - `llm/mod.rs`
3. Проверь adapters:
   - `infrastructure/postgres/*`
   - `infrastructure/qdrant.rs`
   - `infrastructure/gemini*.rs`
4. Проверь миграции:
   - `migrations/0001_initial_schema.sql`
   - `migrations/0002_memory_long_term.sql`
   - `migrations/0003_concurrency_failure_modes.sql`
   - `migrations/0004_memory_embedding_retry_dlq.sql`
5. Проверь текущий контракт API:
   - все endpoints в `crates/api/src/main.rs`.
6. Проверь поведение конкуррентности:
   - `agent_core/tick_runner.rs`.

Ключевые инварианты:
- memory recall работает только для `embedded` записей;
- summary-память создается как отдельная запись и архивирует source;
- mood/state валидны в диапазоне `[-1.0, 1.0]` (clamp в orchestrator);
- duplicate/busy tick защита есть и process-local, и cross-process через Postgres lease/idempotency;
- `embedding_status` цикл: `pending -> processing -> embedded | pending(retry) | dead_letter`.

Что менять осторожно:
- формат payload в `events` (его читает UI/аналитика);
- статусную машину `embedding_status`;
- схему `memory_entries` и согласованность с Qdrant.

---

## 19. Команды разработки

Проверка сборки:
```bash
cargo check --workspace
```

Тесты:
```bash
cargo test --workspace
```

Форматирование:
```bash
cargo fmt
```

---

## 20. Почему сейчас работает именно так

Архитектура сознательно выбрана как **модульный монолит**:
- быстрее собрать на хакатоне;
- проще дебажить end-to-end;
- можно эволюционно выделять сервисы позже.

Текущая версия оптимизирована под "живой прототип backend":
- уже есть core тики, память, базовый real-time канал;
- еще не хватает продуктовых API/безопасности/observability hardening для production-grade.

Если цель - выиграть демо за 24-48 часов, правильный путь:
1. закрыть P0 по списку выше;
2. показать живой сценарий 2-3 агентов с memory recall и user intervention;
3. не распиливать на микросервисы до появления реальной нагрузки.
