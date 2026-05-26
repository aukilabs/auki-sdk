diagnostic-app:
    cargo run -p auki-diagnostic-app

overwatch:
    node examples/overwatch/scripts/stage-sdk.mjs
    npm --prefix examples/overwatch install
    npm --prefix examples/overwatch run dev

overwatch-smoke:
    node examples/overwatch/scripts/stage-sdk.mjs
    npm --prefix examples/overwatch install
    npm --prefix examples/overwatch run smoke

install-toolchain:
    bash scripts/install-toolchain.sh

generate-swift-bindings crate:
    bash scripts/generate-swift-bindings.sh "{{crate}}"
    bash scripts/build-swift-xcframework.sh "{{crate}}"

generate-python-bindings crate:
    bash scripts/generate-python-bindings.sh "{{crate}}"

generate-javascript-bindings crate:
    bash scripts/generate-javascript-bindings.sh "{{crate}}"

binding-api-coverage *crates:
    python3 scripts/bindings/check-binding-api-coverage.py {{crates}}

check-binding-api-coverage *crates:
    python3 scripts/bindings/check-binding-api-coverage.py --fail-on-gaps {{crates}}

generate-rust-proto:
    bash scripts/generate-rust-proto.sh

generate-proto:
    bash scripts/generate-proto.sh

generate-javascript-proto:
    bash scripts/generate-javascript-proto.sh

generate-swift-proto:
    bash scripts/generate-swift-proto.sh

generate-python-proto:
    bash scripts/generate-python-proto.sh
