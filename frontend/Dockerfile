FROM node:20-alpine AS builder

WORKDIR /app

# 1. Копируй package.json И package-lock.json
COPY package*.json ./

# 2. Установи зависимости
RUN npm ci --only=production=false --no-optional && npm cache clean --force

# 3. Скопируй ВСЕ файлы проекта
COPY . .

# 4. Сделай исполняемыми скрипты npm
RUN find node_modules/.bin -type f -exec chmod +x {} + || true

# 5. Собери проект
RUN npm run build

# 6. Runtime nginx
FROM nginx:alpine AS runtime
COPY --from=builder /app/dist /usr/share/nginx/html
# nginx.conf если есть
EXPOSE 3000
CMD ["nginx", "-g", "daemon off;"]
