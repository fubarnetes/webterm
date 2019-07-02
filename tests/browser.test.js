const child_process = require('child_process');
const os = require('os');

describe('Basic tests', () => {
    let server_process;
    beforeAll(() => {
        const searchTerm = "Starting server";
        let stderr = "";
        server_process = child_process.exec('target/debug/webterm-server --command /bin/bash', {
            "env": {
                "RUST_LOG": "actix_net::server::server=info"
            }
        });
        return new Promise((resolve, reject) => {
            let start_failed = () => {
                console.log("Could not start server");
                reject();
            };
            server_process.stderr.on('end', start_failed);
            let data_listener = (data) => {
                stderr += data;
                if (stderr.includes(searchTerm)) {
                    server_process.stderr.removeListener('end', start_failed);
                    server_process.stderr.removeListener('data', data_listener);
                    resolve();
                }
            };
            server_process.stderr.on('data', data_listener);
        });
    });

    it('should load', async () => {
        await expect(page.goto('http://localhost:8080')).resolves.not.toBeNull();
    });

    it('can run echo command', async () => {
        await page.goto('http://localhost:8080');
        let content = await page.evaluate(async () => {
            function sleep(ms) {
                return new Promise(resolve => setTimeout(resolve, ms));
            }

            sock.send('echo teststring\n');
            await sleep(50);

            term.selectAll();
            return term.getSelection();
        });
        await expect(content).toMatch(/echo teststring\n/);
        await expect(content).toMatch(/\nteststring\n/);
    });

    it('passes viewport to client software', async () => {
        await page.goto('http://localhost:8080');
        let viewport = await page.evaluate(() => {
            return {
                cols: term.cols,
                rows: term.rows
            }
        });

        expect(viewport.rows).toBeGreaterThan(0);
        expect(viewport.cols).toBeGreaterThan(0);

        let content = await page.evaluate(async () => {
            function sleep(ms) {
                return new Promise(resolve => setTimeout(resolve, ms));
            }

            sock.send('stty size\n');
            await sleep(50);

            term.selectAll();
            return term.getSelection();
        });
        expect(content).toContain(`${viewport.rows} ${viewport.cols}`);
    });

    it('sends resize events', async () => {
        await page.goto('http://localhost:8080');
        await page.evaluate(async (sigwinch) => {
            function sleep(ms) {
                return new Promise(resolve => setTimeout(resolve, ms));
            }
            await sleep(100);
            sock.send(`trap 'stty size' ${sigwinch}\n`);
        }, os.constants.signals.SIGWINCH);
        await page.setViewport({
            width: 400,
            height: 600
        });
        let content = await page.evaluate(async () => {
            function sleep(ms) {
                return new Promise(resolve => setTimeout(resolve, ms));
            }
            await sleep(100);
            term.selectAll();
            return term.getSelection();
        });
        let viewport = await page.evaluate(() => {
            return {
                cols: term.cols,
                rows: term.rows
            }
        });
        expect(content).toContain(`${viewport.rows} ${viewport.cols}`);
    });

    afterAll(async () => {
        return new Promise((resolve, reject) => {
            server_process.on('exit', (code, signal) => {
                resolve();
            });
            server_process.kill('SIGTERM');
        });
    });
});