# midir browser examples

## Building the example

Use [`wasm-pack`](https://rustwasm.github.io/) to build the WASM artifacts using the [`no-modules`](https://rustwasm.github.io/docs/wasm-bindgen/examples/without-a-bundler.html#using-the-older---target-no-modules) target.

```sh
wasm-pack build --target no-modules
```

The output files will placed in a new directory named `./pkg` by default (expected by `index.html`).

## Running the example

Serve the example directory over HTTP with something like [`serve`](https://github.com/vercel/serve).

```sh
serve .
```

You should now be able to view the example in your browser at something like `http://localhost:3000`.
