<div align="center">

# 🚀 Название проекта

<div align="center">
<img src="https://img.shields.io/badge/Go-00ADD8?style=for-the-badge&logo=go&logoColor=white" alt="Go"/>
<img src="https://img.shields.io/badge/React-20232A?style=for-the-badge&logo=react&logoColor=%2361DAFB" alt="React"/>
<img src="https://img.shields.io/badge/TypeScript-007ACC?style=for-the-badge&logo=typescript&logoColor=white" alt="TypeScript"/>
<img src="https://img.shields.io/badge/JavaScript-F7DF1E?style=for-the-badge&logo=javascript&logoColor=black" alt="JavaScript"/>
</div>

[![CI](https://img.shields.io/github/actions/workflow-status/user/repo/ci.yml?branch=main&label=CI&style=for-the-badge&logo=github-actions&logoColor=white)](https://github.com/user/repo/actions)
[![License: MIT](https://img.shields.io/badge/License-MIT-brightgreen.svg?style=for-the-badge&logo=mit)](https://opensource.org/licenses/MIT)
[![Coverage](https://img.shields.io/badge/coverage-92%25-brightgreen?style=for-the-badge&logo=codecov&logoColor=white)](https://codecov.io/gh/user/repo)

> **Платформа для образовательных хакатонов с real-time коллаборацией через WebSocket**

**🛠️ Языки: Go • React • TypeScript • JavaScript**

</div>


---

## 📋 **Информация о проекте**

| 🔹 **Параметр** | 💡 **Значение** |
|----------------|-----------------|
| **Команда** | TEAM-1 |
| **Участники** | [👥 Состав](CONTRIBUTING.md) |
| **Хакатон** | Название хакатона |
| **Кейс** | Описание задачи |
| **Сроки** | ДД.ММ - ДД.ММ.2025 |
| **Формат** | Онлайн |

---

## 🔧 **Переменные окружения** `(.env)`

> [!TIP|label=🔥 Быстро]
> ```
> cp .env.example .env && code .env
> ```

---

### 🌐 Сервер (Go)

```
PORT=8080
HOST=0.0.0.0
```

### 🗄️ PostgreSQL

```
DATABASE_URL=postgres://postgres:secret123@localhost:5432/project_db
```

### 🔐 JWT Auth

```
JWT_SECRET=your-super-secret-key-change-in-production
```

### 🎨 Frontend (React/TS)

```
REACT_APP_API_URL=http://localhost:8080/api
VITE_API_URL=http://localhost:8080/api
```

🛡️ Redis (WebSocket сессии)

```
REDIS_URL=redis://localhost:6379
```


---

## 🚀 **Быстрый старт**

```
git clone https://github.com/username/project-name.git project-name
cd project-name
cp .env.example .env
docker-compose up -d
```

#### 🌐 Frontend (React): http://localhost:3000
#### 🔧 Backend (Go): curl http://localhost:8080/health


> [!SUCCESS|label=✅ Готово!]
> ```
> 🛠️ Go сервер: :8080 ✅
> 🎨 React UI: :3000 ✅
> 🗄️ DB: :5432 ✅
> 🛡️ Redis: :6379 ✅
> ```

---

## 🛠️ **Установка**

### 🐳 **Docker** *(рекомендуется)*

```
docker-compose up --build
docker-compose logs -f backend
```


### ⚙️ **Backend (Go 1.22+)**

```
cd backend
go mod tidy
go run main.go
```


- - **`main.go` (готовый Go сервер):**

```
package main

import (
"log"
"net/http"
"github.com/gin-gonic/gin"
)

func main() {
r := gin.Default()
```

```
r.GET("/health", func(c *gin.Context) {
    c.JSON(http.StatusOK, gin.H{
        "status": "OK", 
        "lang": "Go 1.22",
        "version": "1.0.0"
    })
})

log.Println("🚀 Go Server starting on :8080")
log.Fatal(http.ListenAndServe(":8080", r))
}
```


### 🎨 **Frontend (React + TypeScript)**

```
cd frontend
npm install
npm run dev # http://localhost:3000
```

---

## 📁 **Структура проекта**

```
📂 project/
├── 📂 backend/ # 🛠️ Go 1.22+
│ ├── main.go # ✅ Go сервер
│ ├── go.mod # Gin + GORM
├── 📂 frontend/ # 🎨 React 18+ + TypeScript
│ ├── src/
│ ├── package.json # React, Vite, Tailwind
├── ⚙️ .env.example
├── 🐳 docker-compose.yml
├── 🤖 .github/workflows/ci.yml
├── 🖼️ assets/
├── 📄 LICENSE
└── 📖 CONTRIBUTING.md
```

---

## 🏗️ **Архитектура**

| 🎯 **Компонент** | 🛠️ **Язык** | 🛠️ **Фреймворк** | 🚀 **Команда** | 🌐 **Порт** |
|------------------|--------------|-------------------|----------------|-------------|
| **Frontend** | TypeScript<br>JavaScript | React 18+, Vite | `npm run dev` | :3000 |
| **Backend** | **Go 1.22** | Gin, GORM | `go run main.go` | :8080 |
| **Database** | SQL | PostgreSQL 16 | Docker | :5432 |
| **Cache** | - | Redis | Docker | :6379 |



---

## 📸 **Демо**

<div align="center">
<table>
<tr>
<td><img src="assets/webUI.png" width="500" /></td>
<td><img src="assets/server-logs.png" width="500" /></td>
</tr>
<tr>
<td colspan="2" align="center">
<img src="assets/demo.gif" width="800" />
</td>
</tr>
</table>
</div>

---

<div align="center">

#### Также мы предоставляем вам тест, отснятый на видео: **link**


## ✅ **Реализованный функционал**

| **Backend (Go)** | **Frontend (React/TS)** | **DevOps** |
|------------------|-------------------------|------------|
| ✅ REST API | ✅ TypeScript UI | ✅ Docker Stack |
| ✅ WebSocket | ✅ Tailwind CSS | ✅ CI/CD Pipeline |
| ✅ JWT Auth | ✅ Vite HMR | ✅ GitHub Actions |
| ✅ GORM ORM | ✅ Responsive Design | ✅ Auto Tests |

</div>


---

## 🤝 **Как внести вклад** 

1. `git checkout -b feature/ваша-идея`
2. `git commit -m "feat(Go/React): описание"`
3. `git push origin feature/ваша-идея`
4. **Pull Request** 🎉

[📖 CONTRIBUTING.md](CONTRIBUTING.md)

---

## 📈 **CI/CD** *(тесты Go + React)*

```
name: 🚀 CI/CD
on: [push, pull_request]
jobs:
test:
runs-on: ubuntu-latest
steps:
- uses: actions/checkout@v4
- name: Test Go Backend
run: cd backend && go test ./...
- name: Test React Frontend
run: cd frontend && npm test
```


---

## 🔮 **Roadmap**

| 📋 **Задача** | 📊 **Статус** |
|--------------|---------------|
| Kubernetes | 🔄 |
| Mobile App (React Native) | ⏳ |
| AI Features | ⏳ |

---

<div align="center">

## 📄 **Лицензия**
[![MIT](https://img.shields.io/badge/license-MIT-blue.svg?style=for-the-badge)](LICENSE)

**Поставьте --> ⭐ Star, если помогло!**

</div>
