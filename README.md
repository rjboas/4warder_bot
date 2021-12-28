# 4warder_bot

A bot designed to forward messages to and from matrix rooms.

## Description

The program does the following:

1. Forwards all messages from an input room to a moderation room.

2. Checks if any messages in the moderation room have been reacted to with a check mark, and if so,

3. Forwards those to an output room

## Usage

The program takes no command-line arguments.
Instead, supply a `4warder.toml` file in the working directory from which you execute the program.

## Contributing
Pull requests are welcome. For major changes, please open an issue first to discuss what you would like to change.

## License
Licensed under either of Apache License, Version 2.0 or MIT license at your option.
