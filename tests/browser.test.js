const child_process = require('child_process');

describe('Basic tests', () => {
    let server_process;
    beforeAll(() => {
        const searchTerm = "Starting server";
        let stderr = "";
        server_process = child_process.exec('target/debug/webterm-server', {
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

    afterAll(async () => {
        return new Promise((resolve, reject) => {
            server_process.on('exit', (code, signal) => {
                resolve();
            });
            server_process.kill('SIGTERM');
        });
    });
});