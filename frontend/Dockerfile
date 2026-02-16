FROM node:20-alpine AS builder

# libc6-compat для Rollup/Vite musl
RUN apk add --no-cache libc6-compat

WORKDIR /app

# 1. Копируй ВСЕ файлы проекта ПЕРВЫМИ
COPY . .

# 2. npm ci ПОСЛЕ копирования (сохраняет права)
RUN npm ci --ignore-scripts=false

# 3. Собери проект
RUN npm run build

FROM nginx:alpine AS runtime
COPY --from=builder /app/dist /usr/share/nginx/html
EXPOSE 3000
CMD ["nginx", "-g", "daemon off;"]
