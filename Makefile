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
	poetry run uvicorn arcan.api:app --port 8000 --host 0.0.0.0 --reload