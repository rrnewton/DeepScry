#!/bin/bash

mb list $* | grep '^mtg' | awk '{ print $1 }' | sort --version-sort | xargs -I {} mb show {}
