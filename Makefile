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

langserve_server:
	poetry run uvicorn arcan.api.langserve.app.server:app --host 0.0.0.0 --port 8080