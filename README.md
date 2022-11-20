# Introduction
This software is for management of text files. It is optimized and tested for ten thousand files. It supports for EPUB, PDF, HTML, image and text files.

# Prerequisites

## backend

### macOS
* `brew install fswatch`
* export LD_LIBRARY_PATH=/usr/lib:/usr/local/lib:/opt/homebrew/lib

### Ubuntu
* `sudo apt install -y fswatch` (in Ubuntu)
* export LD_LIBRARY_PATH=/usr/lib:/usr/lib/x86_64-linux-gnu/libfswatch

### fixing libfswatch.py
* The fswatch module (0.1.1) has a bug in importing libfswatch dynamic module. 
* My unofficial patch for this bug is the following.
* check your python site-packages path by 
  * `python -c "import site; print(site.getsitepackages()[0])"`
* copy backend/fswatch.patched.py to <python site-packages path>/fswatch/fswatch.py
  * ex) `cp backend/fswatch.patched.py ~/.pyenv/versions/tm/lib/python3.10/site-packages/fswatch/fswatch.py`
* copy backend/libfswatch.patched.py to <python site-packages path>/fswatch/libfswatch.py
  * ex) `cp backend/libfswatch.patched.py ~/.pyenv/versions/tm/lib/python3.10/site-packages/fswatch/libfswatch.py`

### common
* `cd backend`
* `cp .env.example .env`
* check .env file and modify environment variables.
* `pip install -r requirements.txt`
* `uvicorn --reload backend/main.app`
    * `nohup uvicorn --reload backend/main.app &`

## frontend
* `cd frontend`
* `cp .env.production.example .env.production`
* check .env.production and modify environment variables.
* `npm install`
* `npm run build`

# License
[LICENSE](LICENSE)
