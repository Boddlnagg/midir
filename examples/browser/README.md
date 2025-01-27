# midir browser examples

## Building the examples

From each example directory, use [`wasm-pack`](https://rustwasm.github.io/) to build the WASM artifacts using the [`web`](https://rustwasm.github.io/docs/wasm-bindgen/examples/without-a-bundler.html) target.

```sh
cd read_input_sync
wasm-pack build --target web
```

The output files will placed in a new directory named `./pkg` by default (expected by `index.html`).

## Running the examples

Serve the example directory over HTTP with something like [`serve`](https://github.com/vercel/serve).

```sh
serve .
```

You should now be able to view the example in your browser at `http://localhost:3000`.
