# Limits

## Значения

### HTTP

| Параметр                    | default | prod   | Ограничивает               |
|-----------------------------|---------|--------|----------------------------|
| `server.request_timeout_ms` | 10 с    | 10 с   | запрос целиком, дальше 408 |
| `api::REQUEST_BODY_LIMIT`   | 64 КиБ  | 64 КиБ | тело запроса, дальше 413   |
| `pagination::PAGE_SIZE`     | 10      | 10     | записей на страницу списка |

### База данных

| Параметр                        | default | prod | Ограничивает                             |
|---------------------------------|---------|------|------------------------------------------|
| `database.max_connections`      | 5       | 4    | соединений в пуле                        |
| `database.acquire_timeout_ms`   | 3 с     | 2 с  | ожидание соединения, дальше `overloaded` |
| `database.statement_timeout_ms` | 5 с     | 5 с  | один запрос, дальше `overloaded`         |

### Файлы

| Параметр                         | default | prod  | Ограничивает                                     |
|----------------------------------|---------|-------|--------------------------------------------------|
| `files.max_file_size_bytes`      | 5 МиБ   | 5 МиБ | одно вложение                                    |
| `files.max_files_per_assignment` | 5       | 5     | вложений у задания                               |
| `files.max_files_per_submission` | 1       | 1     | зарезервировано, сейчас не используется          |
| `files.transfer_timeout_ms`      | 20 с    | 20 с  | одну передачу: ожидание слота, `getFile` и поток |
| `files.max_concurrent_transfers` | 4       | 1     | одновременных передач, а значит пик памяти       |

### Telegram

| Параметр                           | default | prod | Ограничивает                          |
|------------------------------------|---------|------|---------------------------------------|
| `telegram.request_timeout_ms`      | 5 с     | 5 с  | один запрос к Bot API                 |
| `telegram.polling_timeout_ms`      | 10 с    | 20 с | long polling                          |
| `telegram.rate_limit_burst`        | 8       | 6    | updates подряд от одного пользователя |
| `telegram.rate_limit_replenish_ms` | 1 с     | 1 с  | восстановление одного токена          |
| `telegram.max_concurrent_updates`  | 32      | 8    | одновременно обрабатываемых updates   |

### Уведомления о проверке

| Параметр                                | default | prod  | Ограничивает                         |
|-----------------------------------------|---------|-------|--------------------------------------|
| `notification_worker.poll_interval_ms`  | 0,5 с   | 1 с   | паузу на пустой очереди              |
| `notification_worker.lease_duration_ms` | 25 с    | 25 с  | аренду записи на одну доставку       |
| `notification_worker.max_attempts`      | 10      | 10    | попыток до `failed`                  |
| `notification_worker.retry_initial_ms`  | 1 с     | 1 с   | первую паузу, дальше она удваивается |
| `notification_worker.retry_max_ms`      | 5 мин   | 5 мин | потолок паузы                        |

### Остановка

| Параметр                                | default | prod | Ограничивает                                                                                            |
|-----------------------------------------|---------|------|---------------------------------------------------------------------------------------------------------|
| `telegram.shutdown_grace_ms`            | 20 с    | 20 с | вклад бота в grace                                                                                      |
| `notification_worker.shutdown_grace_ms` | 10 с    | 10 с | вклад воркера в grace                                                                                   |
| эффективный grace                       | 20 с    | 20 с | дренирование задач и закрытие пула; максимум из `server.request_timeout_ms` и обоих `shutdown_grace_ms` |
| `stop_grace_period`                     | —       | 30 с | ожидание Docker до SIGKILL                                                                              |

### Окружение production

| Параметр                       | Значение                                                  | Где                            |
|--------------------------------|-----------------------------------------------------------|--------------------------------|
| память и CPU приложения        | 320 МиБ, 1 CPU                                            | `compose.yml`                  |
| память и CPU PostgreSQL        | 448 МиБ, 1 CPU                                            | `compose.yml`                  |
| `max_connections` PostgreSQL   | 20                                                        | `compose.yml`                  |
| health check                   | раз в 10 с, `curl --max-time 5`, unhealthy после 3 неудач | `Dockerfile`                   |
| ожидание блокировки при backup | 10 с                                                      | `tools/ops/backup.sh`          |
| `pg_dump`                      | 5 мин + 10 с до SIGKILL                                   | `BACKUP_TIMEOUT_SECONDS`       |
| готовность после deploy        | 2 мин                                                     | `tools/ops/deploy.sh`          |
| серверный deploy, SSH-шаг, job | 15 мин + 30 с, 17 мин, 20 мин                             | `.github/workflows/deploy.yml` |

## bulkhead

| Отсек                        | Чем задан                         | При переполнении                                                                        |
|------------------------------|-----------------------------------|-----------------------------------------------------------------------------------------|
| память и CPU приложения и БД | `mem_limit`, `cpus`               | OOM-kill только своего контейнера                                                       |
| передачи файлов              | `files.max_concurrent_transfers`  | передача ждёт слот, но не дольше `transfer_timeout_ms`                                  |
| один пользователь            | `telegram.rate_limit_*`           | лишние updates этого пользователя молча отбрасываются                                   |
| обработчики бота             | `telegram.max_concurrent_updates` | чтение updates ждёт слот, задерживая все чаты; ничего не теряется                       |
| пул БД — **общий**           | `database.max_connections`        | HTTP, бот, воркер и `/health/ready` ждут `acquire_timeout_ms`, затем `overloaded` (503) |

## Инварианты

### Проверяются при старте

Нарушение останавливает запуск: `AppConfig::validate`.

| Инвариант                                                                                           | default     | prod        | Иначе                                                       |
|-----------------------------------------------------------------------------------------------------|-------------|-------------|-------------------------------------------------------------|
| `server.request_timeout_ms ≥ acquire_timeout_ms + statement_timeout_ms`                             | 10 ≥ 8 с    | 10 ≥ 7 с    | 408 раньше, чем ответила БД                                 |
| `lease_duration_ms ≥ 2 × (acquire_timeout_ms + statement_timeout_ms) + telegram.request_timeout_ms` | 25 ≥ 21 с   | 25 ≥ 19 с   | аренда истекает посреди доставки, уведомление уходит дважды |
| `files.transfer_timeout_ms ≤ эффективный grace`                                                     | 20 ≤ 20 с   | 20 ≤ 20 с   | остановка обрывает передачу файла                           |
| `rate_limit_burst × rate_limit_replenish_ms ≤ 24 ч`                                                 | 8 с         | 6 с         | лимитер не собирается                                       |
| `retry_max_ms ≥ retry_initial_ms`                                                                   | 5 мин ≥ 1 с | 5 мин ≥ 1 с | —                                                           |
| `max_files_per_assignment ≤ 10`                                                                     | 5           | 5           | клавиатура вложений слишком длинная                         |
| `max_concurrent_updates ≤ 1024`                                                                     | 32          | 8           | —                                                           |
| `polling_timeout_ms` кратен секунде                                                                 | 10 с        | 20 с        | Bot API принимает timeout в секундах                        |

### Проверяются вручную

Процесс их не видит: сверять при правке `compose.yml`, `Dockerfile` или переезде.

| Инвариант                                                            | Сейчас             | Иначе                                   |
|----------------------------------------------------------------------|--------------------|-----------------------------------------|
| timeout health check > `acquire_timeout_ms`                          | 5 > 2 с            | unhealthy при исправном приложении      |
| `max_connections` PostgreSQL ≥ пул + запас на `psql` и `pg_dump`     | 20 ≥ 4             | backup не подключится под нагрузкой     |
| память приложения ≫ `max_concurrent_transfers × max_file_size_bytes` | 320 ≫ 5 МиБ        | OOM-kill на пике передач                |
| `stop_grace_period` > эффективный grace                              | 30 > 20 с          | SIGKILL до завершения задач             |
| серверный deploy + 30 с < SSH-шаг < job                              | 15,5 < 17 < 20 мин | runner не дожидается серверного cleanup |

## Под нагрузкой

| Ситуация                              | Признак                                                             | Комментарий                                         |
|---------------------------------------|---------------------------------------------------------------------|-----------------------------------------------------|
| пул исчерпан                          | `overloaded`, 503, `kind=pool_acquire_timeout`, `/health/ready` 503 | проходит само; Docker по unhealthy не перезапускает |
| медленный запрос                      | `overloaded`, 503, `kind=statement_timeout`                         | само не пройдёт, запрос надо разбирать              |
| Telegram недоступен                   | уведомление получает `failed` примерно через 9 мин                  | паузы 1 + 2 + … + 256 с плюс до 10 запросов по 5 с  |
| бот упёрся в `max_concurrent_updates` | ответы задерживаются во всех чатах                                  | задержка вместо потери updates                      |
