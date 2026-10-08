FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p uet-server

FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=build /src/target/release/uet-server /uet-server
ENV BIND=0.0.0.0:8080
EXPOSE 8080
ENTRYPOINT ["/uet-server"]
