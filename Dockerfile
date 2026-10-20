# The headless server as a container. Built by the release workflow from the
# already-compiled static (musl) binaries, one per architecture, so the image
# is just the binary on a minimal base with CA certificates.
#
#   docker run -p 8740:8740 -v librebooks:/data \
#     -e LIBREBOOKS_PUBLIC_URL=https://books.example.com ghcr.io/cinderblock/librebooks
#
# A container never updates itself: pull a new image instead.
FROM gcr.io/distroless/static-debian12:nonroot

ARG TARGETARCH
COPY dist/${TARGETARCH}/librebooks /usr/local/bin/librebooks

ENV LIBREBOOKS_DATA_DIR=/data \
    LIBREBOOKS_BIND=0.0.0.0:8740 \
    LIBREBOOKS_IN_CONTAINER=1
VOLUME ["/data"]
EXPOSE 8740
USER nonroot
ENTRYPOINT ["/usr/local/bin/librebooks"]
CMD ["serve"]
