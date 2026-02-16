FROM node:20-alpine AS builder

WORKDIR /app

# 1. Установи зависимости
COPY package*.json ./
RUN npm ci \
    && npm cache clean --force

# 2. ИСПРАВЬ права на исполняемые файлы
RUN chmod +x node_modules/.bin/*

# 3. Скопируй исходники и собери
COPY . .
RUN npm run build

FROM nginx:alpine AS runtime
COPY --from=builder /app/dist /usr/share/nginx/html
COPY nginx.conf /etc/nginx/conf.d/default.conf
EXPOSE 3000
CMD ["nginx", "-g", "daemon off;"]
