attribute vec2 pos;

uniform vec4 view;

void main() {
    gl_Position = vec4(
        // clowntown
        vec2(-1, 1) + vec2(2, -2) * (pos - vec2(view.x, view.y)) / vec2(view.z, view.w),
        0, 1
    );
}
