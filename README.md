# Introduction
This software is for management of text files. It is optimized and tested for ten thousand files. It supports for EPUB, PDF, HTML, image and text files.

# Prerequisites

## backend
* `cd backend`
* `cp .env.example .env`
* check .env file and modify environment variables.
* `brew install fswatch`
* `pip install -r requirements`
* uvicorn --reload backend/main.app

## frontend
* `cd frontend`
* `cp .env.production.example .env.production`
* check .env.production and modify environment variables.
* `npm install`
* `npm run build`

# License
[LICENSE](LICENSE)