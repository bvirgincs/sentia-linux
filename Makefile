SHELL := /usr/bin/env bash

.PHONY: bootstrap packages repo iso test test-iso test-install test-failure release clean

bootstrap:
	./build/bootstrap/bootstrap.sh

packages:
	./build/scripts/packages.sh

repo:
	./build/scripts/repo.sh

iso:
	./build/scripts/iso.sh

test:
	./build/scripts/test.sh

test-iso:
	./build/scripts/test-iso.sh

test-install:
	./build/scripts/test-install.sh

test-failure:
	./build/scripts/test-failure.sh

release: bootstrap packages repo iso test

clean:
	./build/scripts/clean.sh
