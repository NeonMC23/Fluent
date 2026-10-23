from backend.app import create_app, load_config


if __name__ == "__main__":
    config = load_config()
    app = create_app(config)
    app.run(host=config["host"], port=int(config["port"]), debug=False)
