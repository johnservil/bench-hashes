# Checking the map's and the guide's scripts

`map.js` drives a generated `bench-hashes.map.html` in Chromium: every chart
opens in place and closes, with its headers lit and the values under the
pointer; a header greys its charts; every call name links to its
documentation; a link to one chart opens it. `guide.js` walks every route of
`bench-hashes.guide.html`, and `guide-summary.js` checks the guide's claims
on synthetic data.

    npm install playwright
    node tools/graph-check/map.js benchmark-results/<machine>/bench-hashes.map.html /usr/bin/chromium
    node tools/graph-check/guide.js benchmark-results/<machine>/bench-hashes.guide.html /usr/bin/chromium
    node tools/graph-check/guide-summary.js /usr/bin/chromium
