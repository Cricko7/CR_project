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
| 2. Эмоциональный интеллект | `FULLY IMPLEMENTED` | Реализована динамика эмоций на каждом тике (summary + personality traits -> valence/arousal/mood), а также периодический mood decay к нейтрали в worker |
| 3. Архитектура агента (рефлексия→цель→действие) | `FULLY IMPLEMENTED` | Tick pipeline декомпозирован на 4 стадии: reflection, goal selection, action planning, execution + side effects; результат и stage-trace пишутся в event payload |
| 4. Мультиагентность (общение, отношения) | `FULLY IMPLEMENTED` | Реализованы API отправки/чтения сообщений, auto-seeding случайных разговоров (в т.ч. onboarding новых агентов), worker delivery lifecycle (`queued -> processing -> delivered/failed`) и автоматическое обновление `relationships` (affinity + history) при доставке |
| 5. Real-time dashboard life feed | `FULLY IMPLEMENTED` | Реализован единый live feed через `/ws/events` для событий из API и worker (DB tail bridge), а также cursor polling через `/events?after_id=...` |
| 6. Граф отношений | `FULLY IMPLEMENTED` | Есть graph snapshot API (`/relationships/graph`) и live stream (`/ws/relationships`); worker публикует `agent.relationship.updated` события |
| 7. Inspector агента | `FULLY IMPLEMENTED` | Есть агрегирующий endpoint `/agents/{id}/inspector` (agent profile + state + recent events/messages/relationships/memories + optional recall) |
| 8. Панель вмешательства | `FULLY IMPLEMENTED` | Реализованы `POST/GET /interventions` (включая `append_event`, `send_message`, `set_time_scale`) и прямой runtime-control скорости через `GET/POST /simulation/time-scale` |
| Доп. фича 1: настроение влияет на стиль речи | `FULLY IMPLEMENTED` | Введены формализованные mood-based speech style policies (tone/cadence/diction/punctuation), они инжектятся в LLM prompts, применяются в deterministic fallback и сохраняются в `events.payload_json.speech_style` |
| Доп. фича 2: страница агента с историей отношений | `FULLY IMPLEMENTED` | Добавлен агрегированный relationship timeline API (`/agents/{id}/relationships/history`) и расширен inspector (`relationship_timeline`), что закрывает backend-контракт для страницы истории отношений |

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
  - staged decision pipeline: reflection -> goal -> action_plan -> execution;
  - mood-based speech style policy layer для текста стадий и execution summary;
  - запись `agent_states` и `events`;
  - LLM-driven stage outputs с per-stage fallback.

### Memory
- Файлы: `crates/sim-backend/src/memory/*`
- Ответственность:
  - append episodic memory;
  - embedding pipeline (pending -> embedded/failed);
  - recall по векторному поиску;
  - overflow summarization и архивирование старых записей.

### Communication
- Текущее состояние:
  - есть enqueue/list API для межагентных сообщений;
  - worker conversation seeder автоматически инициирует случайные разговоры между 2-3 агентами;
  - при появлении нового агента worker автоматически стартует onboarding-диалог с другими агентами (включая 3-сторонний сценарий при наличии двух peers);
  - worker delivery loop обрабатывает очередь сообщений и пишет delivery events;
  - при доставке сообщения обновляется relationship score/history между агентами.

### Emotions
- Текущее состояние:
  - `valence/arousal/mood_label` обновляются на каждом тике через emotional model (action summary + personality bias);
  - mood напрямую задает speech style profile (tone/cadence/diction/punctuation) для generation/fallback;
  - отдельный worker выполняет периодический mood decay к нейтральному состоянию.

### Interventions
- Текущее состояние:
  - есть полноценный API панели вмешательства: `POST /interventions`, `GET /interventions`;
  - поддержаны action-типы: `trigger_tick`, `append_memory`, `send_message`, `append_event`, `set_time_scale`;
  - есть отдельный endpoint управления скоростью симуляции: `GET/POST /simulation/time-scale`;
  - все запросы фиксируются в `interventions` с `result_status=applied|failed`.

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
├── Dockerfile                      # unified runtime image (api/worker)
├── docker-compose.yml              # one-command full stack run
├── docker/
│   └── postgres/
│       └── seed_agents.sql         # auto-seed demo agents
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
│       │   ├── 0004_memory_embedding_retry_dlq.sql
│       │   ├── 0005_multyagent_communication.sql
│       │   └── 0006_simulation_time_scale.sql
│       └── src/
│           ├── app/                # config, runtime, observability
│           ├── agent_core/         # orchestrator, persistence traits, tick runner
│           ├── memory/             # memory service + ports
│           ├── infrastructure/     # postgres, qdrant, gemini adapters
│           ├── llm/                # LLM port
│           └── lib.rs
|
├── frontend/
│   ├── Dockerfile
│   ├── nginx.conf
│   ├── package.json
│   ├── tailwind.config.js
│   ├── tsconfig.json
│   ├── vite.config.ts
│   ├── index.html
│   ├── public/
│   │   └── favicon.ico
│   └── src/
│       ├── index.css
│       ├── main.tsx
│       ├── App.tsx
│       ├── types/api.ts
│       ├── hooks/useCyberLife.ts
│       └── components/
│           ├── Dashboard.tsx
│           ├── AgentCard.tsx
│           ├── AgentInspector.tsx
│           ├── Graph.tsx
│           └── InterventionPanel.tsx
|
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
   - прогоняет decision pipeline по стадиям:
     - reflection;
     - goal selection;
     - action planning;
     - execution + side effects;
   - пересчитывает `valence/arousal/mood_label` на основе summary и personality traits;
   - upsert state;
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
3. API-процесс запускает `event_bridge_worker`, который tail-ит таблицу `events` из Postgres по курсору `events.id`.
4. Bridge публикует новые события в in-memory `broadcast` hub и WS-клиенты получают их в real-time.
5. Клиент при необходимости фильтрует/запрашивает только одного агента через `agent_id`.
6. При лаге клиента соединение закрывается с ошибкой stream lagged.

### 7.5 Message/relationship flow
1. Источник сообщения:
   - API `POST /agents/{receiver_id}/messages`, или
   - auto-seeder воркера (random conversation/onboarding новых агентов).
2. Сообщение добавляется в `messages` со статусом `queued`.
3. Worker `message_delivery_worker` claim'ит queued batch (`FOR UPDATE SKIP LOCKED`) и переводит в `processing`.
4. Для каждой записи:
   - пишется event `agent.message.received` для receiver;
   - обновляется `relationships` (upsert pair + affinity delta + history append).
5. Delivery статус:
   - success -> `delivered`;
   - failure -> `failed` + `delivery_error`.

### 7.6 Relationship graph feed flow
1. Worker при изменении affinity/history пишет доменное событие `agent.relationship.updated` в `events`.
2. API `event_bridge_worker` tail-ит `events` и публикует relationship updates в WS-hub.
3. Dashboard может:
   - получать snapshot графа через `GET /relationships/graph`;
   - получать live edge updates через `GET /ws/relationships`.

### 7.7 Inspector flow
1. Клиент вызывает `GET /agents/{id}/inspector`.
2. API параллельно читает: profile/state/events/messages/relationships/memory и message-based relationship timeline.
3. В ответе возвращаются блоки `recent_relationships` и `relationship_timeline` (история взаимодействий с контрагентами + snapshot текущей связи).
4. При наличии `recall_query` API дополнительно запускает vector recall и включает блок релевантных memories.
5. Dashboard получает готовый агрегированный срез для страницы агента одним запросом.

### 7.8 Intervention flow
1. Клиент панели вызывает `POST /interventions` с `admin_user_id` и `action`.
2. API выполняет действие (`tick`, `memory append`, `message enqueue`, `event append`).
3. Результат и payload действия сохраняются в таблице `interventions`.
4. История действий читается через `GET /interventions`.

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

### 8.3.1 Agent Inspector Profile

#### `GET /agents/{id}/inspector?events_limit=<1..200>&messages_limit=<1..200>&relationships_limit=<1..200>&timeline_limit=<1..200>&memories_limit=<1..200>&recall_query=...&recall_top_k=<1..50>`

Response:
```json
{
  "agent": {
    "id": "uuid",
    "name": "Alice",
    "avatar_url": null,
    "personality_json": {"traits":["curious","friendly"]},
    "created_at": "2026-02-16T12:00:00Z"
  },
  "state": {
    "agent_id": "uuid",
    "mood_label": "calm",
    "valence": 0.22,
    "arousal": -0.04,
    "updated_at": "2026-02-16T12:04:00Z"
  },
  "summary": {
    "events_count": 20,
    "messages_count": 10,
    "relationships_count": 5,
    "timeline_count": 20,
    "memories_count": 20
  },
  "recent_events": [...],
  "recent_messages": [...],
  "recent_relationships": [...],
  "relationship_timeline": [...],
  "recent_memories": [...],
  "recall": {
    "query": "recent conflict",
    "top_k": 8,
    "items": [...]
  }
}
```

### 8.4 Events feed

#### `GET /events?agent_id=<uuid>&limit=<1..200>&after_id=<event_id>`

Поведение:
- без `after_id`: возвращаются последние события (как раньше), `ORDER BY occurred_at DESC`;
- с `after_id`: cursor-mode для dashboard polling, возвращаются события `id > after_id`, `ORDER BY id ASC`, и выдается `next_after_id`.

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
  ],
  "next_after_id": 1
}
```

### 8.4.1 Interventions

#### `POST /interventions`

Request:
```json
{
  "admin_user_id": "demo-admin",
  "action": {
    "type": "send_message",
    "sender_agent_id": "uuid-a",
    "receiver_agent_id": "uuid-b",
    "content": "Hold position and report status."
  }
}
```

Поддерживаемые `action.type`:
- `trigger_tick`
- `append_memory`
- `send_message`
- `append_event`
- `set_time_scale`

Response (`200`):
```json
{
  "intervention": {
    "id": 21,
    "admin_user_id": "demo-admin",
    "action_type": "send_message",
    "payload_json": {"action": {...}, "effect": {...}},
    "result_status": "applied",
    "created_at": "2026-02-16T12:00:00Z"
  },
  "effect": {
    "type": "message",
    "message_id": 77,
    "status": "queued"
  }
}
```

#### `GET /interventions?limit=<1..200>`

Response:
```json
{
  "items": [
    {
      "id": 21,
      "admin_user_id": "demo-admin",
      "action_type": "send_message",
      "payload_json": {"action": {...}, "effect": {...}},
      "result_status": "applied",
      "created_at": "2026-02-16T12:00:00Z"
    }
  ]
}
```

### 8.4.2 Simulation time scale

#### `GET /simulation/time-scale`

Response (`200`):
```json
{
  "time_scale": 1.0,
  "updated_at": "2026-02-16T12:00:00Z"
}
```

#### `POST /simulation/time-scale`

Request:
```json
{
  "time_scale": 1.5
}
```

Response (`200`):
```json
{
  "time_scale": 1.5,
  "updated_at": "2026-02-16T12:05:00Z"
}
```

Validation:
- `time_scale` должен быть числом в диапазоне `[0.1, 10.0]`.
- Значение применяется без перезапуска и используется worker-циклами симуляции.

### 8.5 Agent message send

#### `POST /agents/{receiver_id}/messages`

Request:
```json
{
  "sender_agent_id": "uuid",
  "content": "Let's cooperate on exploring the market."
}
```

Response (`202`):
```json
{
  "message_id": 77,
  "status": "queued"
}
```

### 8.6 Agent messages list

#### `GET /agents/{id}/messages?limit=<1..200>`

Response:
```json
{
  "items": [
    {
      "id": 77,
      "sender_type": "agent",
      "sender_id": "uuid",
      "receiver_agent_id": "uuid",
      "content": "Let's cooperate on exploring the market.",
      "status": "delivered",
      "created_at": "2026-02-16T12:00:00Z"
    }
  ]
}
```

### 8.7 Agent relationships

#### `GET /agents/{id}/relationships?limit=<1..200>`

Response:
```json
{
  "items": [
    {
      "id": 5,
      "agent_a": "uuid-a",
      "agent_b": "uuid-b",
      "affinity_score": 0.32,
      "history_summary": "Let's cooperate on exploring the market.",
      "last_interaction_at": "2026-02-16T12:00:01Z",
      "created_at": "2026-02-16T12:00:01Z"
    }
  ]
}
```

### 8.7.1 Agent relationship history timeline

#### `GET /agents/{id}/relationships/history?limit=<1..200>`

Response:
```json
{
  "agent_id": "uuid-a",
  "items": [
    {
      "message_id": 77,
      "direction": "outgoing",
      "counterpart_agent_id": "uuid-b",
      "counterpart_name": "Bob",
      "counterpart_avatar_url": null,
      "content": "Let's cooperate on exploring the market.",
      "status": "delivered",
      "created_at": "2026-02-16T12:00:00Z",
      "relationship": {
        "id": 5,
        "agent_a": "uuid-a",
        "agent_b": "uuid-b",
        "affinity_score": 0.32,
        "history_summary": "Let's cooperate on exploring the market.",
        "last_interaction_at": "2026-02-16T12:00:01Z",
        "created_at": "2026-02-16T12:00:01Z"
      }
    }
  ]
}
```

### 8.8 Relationship graph snapshot

#### `GET /relationships/graph?agent_id=<uuid>&limit_edges=<1..500>`

Response:
```json
{
  "nodes": [
    {
      "agent_id": "uuid-a",
      "name": "Alice",
      "avatar_url": null
    },
    {
      "agent_id": "uuid-b",
      "name": "Bob",
      "avatar_url": null
    }
  ],
  "edges": [
    {
      "id": 5,
      "agent_a": "uuid-a",
      "agent_b": "uuid-b",
      "affinity_score": 0.32,
      "history_summary": "Let's cooperate on exploring the market.",
      "last_interaction_at": "2026-02-16T12:00:01Z",
      "created_at": "2026-02-16T12:00:01Z"
    }
  ]
}
```

### 8.9 Memory append

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

### 8.10 Memory recall

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

### 8.11 Manual summarization

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

### 8.12 Manual embedding processing

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

### 8.13 Dead-letter embeddings

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

### 8.14 WebSocket events

#### `GET /ws/events?agent_id=<uuid>&snapshot_limit=<1..200>`

Server event envelope (`snake_case`, `type` discriminator):
- `snapshot`
- `event_appended`
- `tick_skipped`
- `error`

Examples:
```json
{"type":"snapshot","items":[...]}
```

```json
{
  "type":"event_appended",
  "item": {
    "id": 123,
    "agent_id": "uuid",
    "event_type": "agent.tick.executed",
    "description": "Agent `Alice` executed tick ...",
    "payload": "{\"tick_id\":\"...\"}",
    "occurred_at": "2026-02-16T12:00:00Z"
  }
}
```

### 8.15 WebSocket relationship graph stream

#### `GET /ws/relationships?agent_id=<uuid>&snapshot_limit=<1..500>`

Server event envelope (`snake_case`, `type` discriminator):
- `snapshot`
- `edge_updated`
- `error`

Examples:
```json
{
  "type":"snapshot",
  "graph":{
    "nodes":[...],
    "edges":[...]
  }
}
```

```json
{
  "type":"edge_updated",
  "edge":{
    "id": 5,
    "agent_a": "uuid-a",
    "agent_b": "uuid-b",
    "affinity_score": 0.40,
    "history_summary": "Let's cooperate | Thanks for support",
    "last_interaction_at": "2026-02-16T12:05:01Z",
    "created_at": "2026-02-16T12:00:01Z"
  }
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
- `crates/sim-backend/migrations/0005_multyagent_communication.sql`
- `crates/sim-backend/migrations/0006_simulation_time_scale.sql`

Основные таблицы:
- `agents`: карточка агента, personality JSON.
- `agent_states`: valence/arousal/mood.
- `events`: доменные события.
- `memory_entries`: эпизодическая и summary память.
- `relationships`: affinity/history между агентами, обновляется из message delivery.
- `messages`: очередь межагентных сообщений + delivery status machine.
- `interventions`: журнал админ-вмешательств (action payload + `result_status`).
- `simulation_controls`: singleton runtime-настройка `time_scale` для скорости симуляции.
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
| `API_EVENT_BRIDGE_INTERVAL_MS` | `500` |
| `API_EVENT_BRIDGE_BATCH_SIZE` | `128` |

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
| `GEMINI_MIN_REQUEST_INTERVAL_MS` | `1000` |

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
| `WORKER_MOOD_DECAY_STEP` | `0.06` |
| `WORKER_MESSAGE_INTERVAL_MS` | `1000` |
| `WORKER_MESSAGE_BATCH_SIZE` | `32` |
| `WORKER_CONVERSATION_SCAN_INTERVAL_MS` | `3000` |
| `WORKER_CONVERSATION_MIN_INTERVAL_MS` | `12000` |
| `WORKER_CONVERSATION_MAX_INTERVAL_MS` | `45000` |
| `WORKER_CONVERSATION_AGENT_LIMIT` | `512` |

---

## 11. Локальный запуск (с нуля)

### 11.0 One-command Docker запуск (рекомендуется)

Требования:
- Docker Desktop (или Docker Engine + Compose plugin).

Одна команда, чтобы поднять весь стек:
```bash
docker compose up --build -d
```

Что поднимется:
- `postgres` (DB)
- `qdrant` (vector DB)
- `api` (HTTP + WS)
- `seed` (одноразовый импорт 2 агентов: Alice/Bob)
- `worker` (тики, message delivery, embedding, summarization, mood decay)

Быстрая проверка:
```bash
curl http://127.0.0.1:8080/health
curl "http://127.0.0.1:8080/events?limit=5"
```

Остановить и удалить контейнеры:
```bash
docker compose down
```

Остановить с удалением volume данных:
```bash
docker compose down -v
```

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
set API_EVENT_BRIDGE_INTERVAL_MS=500
set API_EVENT_BRIDGE_BATCH_SIZE=128
set WORKER_AGENT_IDS=<uuid1>,<uuid2>
set WORKER_TICK_CONCURRENCY=8
set WORKER_MOOD_DECAY_STEP=0.06
set WORKER_MESSAGE_INTERVAL_MS=1000
set WORKER_MESSAGE_BATCH_SIZE=32
set GEMINI_API_KEY=<your_google_api_key>
set GEMINI_MIN_REQUEST_INTERVAL_MS=1000
```

Если `GEMINI_API_KEY` не задан, система работает в fallback-режиме:
- tick summaries deterministic;
- embeddings через local hash embedder.

`GEMINI_MIN_REQUEST_INTERVAL_MS` задает минимальный интервал между Gemini запросами
в рамках одного процесса (API или worker). Установите `0`, чтобы отключить throttling.

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
2. **Нет rate limiting** на дорогие endpoint'ы (`/ticks`, `/messages/*`, `/memories/*`, `/process-embeddings`).
3. **Нет tenant isolation** (single-world assumptions).
4. **WS feed без durable delivery**: нет клиентских ack/offset storage, при disconnect нужен reconnect + catch-up.
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
2. **Durable client delivery**: WS feed не хранит пер-клиент offsets/acks; при disconnect нужен reconnect + snapshot/cursor catch-up.

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
| ADR-007 | WS feed через API hub + DB tail bridge | Простая real-time доставка событий из API и worker без отдельного брокера | Нет durable delivery и client ack/offset storage |
| ADR-008 | Trait ports для repo/vector/llm | Тестируемость и сменяемость адаптеров | Больше кода и интерфейсных слоев |

---

## 16. Что уже сделано в коде

1. Базовый runtime и graceful shutdown.
2. Structured logging.
3. Конфигурирование через env с валидацией.
4. Agent tick orchestrator со staged decision pipeline (reflection -> goal -> action_plan -> execution) и mood-based speech style policies.
5. Постгрес-репозиторий для agent core.
6. Memory service:
   - append;
   - embedding pipeline с retry/backoff + DLQ;
   - vector recall;
   - overflow summarization.
7. Qdrant adapter и Gemini adapters.
8. REST endpoints для state/events/memory/ticks/messages/relationships + relationship history timeline + relationship graph snapshot + agent inspector profile + interventions panel.
9. WebSocket endpoints `/ws/events` и `/ws/relationships` с snapshot + live updates.
10. Worker циклы: ticks, mood-decay, message delivery, embedding, summarization.
11. Миграции для базовой схемы, long-term memory, concurrency/failure hardening и мультиагентной коммуникации.
12. Unit tests для `agent_core` и `memory`.

---

## 17 Команды разработки

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
