#!/bin/sh
# F3: the process that watches has room for 4096 connections, and each worker keeps up to
# 127 for as long as it lives: with 33 workers or more, asks wait for ever.
cd /x/min; . ./env.sh
WORKERS=40 start ./m
echo "supervisor $SUP, $(workers | wc -w) workers"
python3 f3.py $SUP
echo "supervisor: $(stat $SUP)"
