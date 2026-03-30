DEPLOY := ./deploy.sh

.PHONY: deploy-audio deploy-led deploy-all push-audio push-led push-all \
        build-audio build-led build-all \
        services status logs restart-all stop-all start-all

# Build + deploy (build, stop, copy, start)
deploy-audio:
	$(DEPLOY) deploy audio

deploy-led:
	$(DEPLOY) deploy led_controller

deploy-all:
	$(DEPLOY) deploy all

# Push only (stop, copy, start — no build)
push-audio:
	$(DEPLOY) push audio

push-led:
	$(DEPLOY) push led_controller

push-all:
	$(DEPLOY) push all

# Build only (no deploy)
build-audio:
	$(DEPLOY) build audio

build-led:
	$(DEPLOY) build led_controller

build-all:
	$(DEPLOY) build all

# Service management
services:
	$(DEPLOY) services

status:
	$(DEPLOY) status

logs:
	$(DEPLOY) logs

restart-all:
	$(DEPLOY) restart

stop-all:
	$(DEPLOY) stop all

start-all:
	$(DEPLOY) start all
