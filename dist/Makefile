compose := sh bin/compose.sh
wait := --wait --wait-timeout 120

.PHONY: help status logs docker-logs psql queues backup prune restart bootstrap deploy restore cron

help:
	@echo 'make status        release, running image, last backup, containers, disk'
	@echo 'make logs          follow the application log'
	@echo 'make docker-logs   container output, including errors before the log starts'
	@echo 'make psql          psql shell'
	@echo 'make queues        stale reviews and stuck notifications'
	@echo 'make backup        dump the database into backups/'
	@echo 'make prune         delete backups older than 14 days, old releases and images'
	@echo 'make restart       recreate the app with the deployed image'
	@echo 'make bootstrap     apply config/bootstrap.json'
	@echo 'make deploy DIR=releases/<version>'
	@echo 'make restore DUMP=backups/<file>.dump CONFIRM=yes'
	@echo 'make cron          nightly backup and prune at 17:00 server time'

status:
	@sh bin/status.sh

logs:
	tail -n 100 -F logs/hw-bot.$$(date -u +%F).log logs/hw-bot.$$(date -u -d tomorrow +%F).log

docker-logs:
	$(compose) logs --tail 100 app database

psql:
	$(compose) exec database psql -U hw_bot hw_bot

queues:
	$(compose) exec -T database psql -U hw_bot -q hw_bot < queues.sql

backup:
	sh bin/backup.sh

prune:
	last=$$(cat backups/.last-success 2>/dev/null || true); \
	find backups -name 'hw-bot-*.dump*' -mtime +14 \
		! -name "$${last##*/}" ! -name "$${last##*/}.sha256" -delete
	ls -dt releases/*/ | tail -n +4 | xargs -r rm -rf
	docker image prune -f

restart:
	$(compose) up -d --no-deps --force-recreate $(wait) app

bootstrap:
	chgrp 10001 config/production.toml config/bootstrap.json
	chmod 640 config/production.toml config/bootstrap.json
	$(compose) up -d --no-recreate $(wait) database
	$(compose) -f compose.bootstrap.yml up -d --no-deps --force-recreate $(wait) app
	$(compose) up -d --no-deps --force-recreate $(wait) app

deploy:
	$(if $(DIR),,$(error usage: make deploy DIR=releases/<version>))
	cd $(DIR) && jq -r '.archive_sha256 + "  deploy.tar.gz"' release.json | sha256sum --check
	tar -xzf $(DIR)/deploy.tar.gz -C $(DIR) --strip-components=1
	bash $(DIR)/bin/deploy.sh . $(DIR)

restore:
	$(if $(DUMP),,$(error usage: make restore DUMP=backups/<file>.dump CONFIRM=yes))
	$(if $(filter yes,$(CONFIRM)),,$(error restore replaces the whole database, add CONFIRM=yes))
	cd $(dir $(DUMP)) && sha256sum --check $(notdir $(DUMP)).sha256
	sh bin/backup.sh
	$(compose) stop app
	$(compose) exec -T database dropdb -U hw_bot --force hw_bot
	$(compose) exec -T database createdb -U hw_bot hw_bot
	$(compose) exec -T database pg_restore -U hw_bot -d hw_bot --no-owner --exit-on-error < $(DUMP)
	$(compose) up -d --no-deps --force-recreate $(wait) app

cron:
	echo '0 17 * * * root make -C $(CURDIR) backup prune >> $(CURDIR)/backups/cron.log 2>&1' > /etc/cron.d/hw-bot-backup
