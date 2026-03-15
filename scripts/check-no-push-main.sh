#!/bin/bash
branch=$(git symbolic-ref HEAD | sed 's|refs/heads/||')
if [ "$branch" = "main" ]; then
  echo "Error: mainブランチへの直接pushは禁止されています。PRを使用してください。"
  exit 1
fi
