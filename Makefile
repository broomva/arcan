SHELL=/bin/bash
devops_state = main
working_dir = `pwd`

install: local_build_and_deploy

local_deploy: build && deploy

rerun: 
	make build \
	&& yarn dev 

build:
	black . \
	&& poetry build \
	&& poetry export -f requirements.txt --output requirements.txt \
	&& poetry install \
	&& yarn build \
	&& yarn install

deploy: 
	modal deploy arcan

local_build_and_deploy: 
	pip uninstall arcan -y \
	&& poetry install \
	&& arcan

package_build:
	python -m build

package_list:
	unzip -l dist/*.whl  

serve:
	poetry run uvicorn arcan.forge:app --port 8000 --host 0.0.0.0 --reload

chainlit:
	poetry run chainlit run arcan/ai/interface/app.py --port 8100 --watch


SHELL := /bin/bash

# Variables definitions
# -----------------------------------------------------------------------------

ifeq ($(TIMEOUT),)
TIMEOUT := 60
endif

# Target section and Global definitions
# -----------------------------------------------------------------------------
.PHONY: all clean test install run deploy down

all: clean test install run deploy down

test:
	poetry run pytest tests -vv --show-capture=all

install: generate_dot_env
	pip install --upgrade pip
	pip install poetry
	poetry install --with dev

run:
	PYTHONPATH=app/ poetry run uvicorn main:app --reload --host 0.0.0.0 --port 8080
	
migrate:
	poetry run alembic upgrade head

deploy: generate_dot_env
	docker-compose build
	docker-compose up -d

up: 
	docker-compose up
down:
	docker-compose down

generate_dot_env:
	@if [[ ! -e .env ]]; then \
		cp .env.example .env; \
	fi

clean:
	@find . -name '*.pyc' -exec rm -rf {} \;
	@find . -name '__pycache__' -exec rm -rf {} \;
	@find . -name 'Thumbs.db' -exec rm -rf {} \;
	@find . -name '*~' -exec rm -rf {} \;
	rm -rf .cache
	rm -rf build
	rm -rf dist
	rm -rf *.egg-info
	rm -rf htmlcov
	rm -rf .tox/
	rm -rf docs/_build