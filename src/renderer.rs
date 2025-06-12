use nalgebra::Matrix4;
use glow::{Context, HasContext as _};

pub struct GpuLines {
    program: glow::Program,
    vao:     glow::VertexArray,
    vbo:     glow::Buffer,
    vertex_count: i32,
    u_mvp:   glow::UniformLocation,
}

unsafe impl Send for GpuLines {}
unsafe impl Sync for GpuLines {}

impl GpuLines {
    pub unsafe fn new(gl: &Context) -> Self {
        let program = {
            let vs = gl.create_shader(glow::VERTEX_SHADER).unwrap();
            gl.shader_source(vs, r#"#version 300 es
                precision highp float;
                uniform mat4 u_mvp;
                layout(location = 0) in vec3 a_pos;
				layout(location = 1) in vec3 a_col;
				out vec3 v_col;
                void main() {
					v_col      = a_col;
					gl_Position = u_mvp * vec4(a_pos, 1.0);
				}"#);
            gl.compile_shader(vs);

            let fs = gl.create_shader(glow::FRAGMENT_SHADER).unwrap();
            gl.shader_source(fs, r#"#version 300 es
                precision mediump float;
				in  vec3 v_col;
				out vec4 o_col;
				void main() { o_col = vec4(v_col, 1.0); }"#);
            gl.compile_shader(fs);

            let prog = gl.create_program().unwrap();
            gl.attach_shader(prog, vs);
            gl.attach_shader(prog, fs);
            gl.link_program(prog);
            gl.delete_shader(vs);
            gl.delete_shader(fs);
            prog
        };

        let vao = gl.create_vertex_array().unwrap();
        let vbo = gl.create_buffer().unwrap();

        gl.bind_vertex_array(Some(vao));
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
        gl.enable_vertex_attrib_array(0);
        gl.vertex_attrib_pointer_f32(0,3,glow::FLOAT,false,24,0);
		gl.enable_vertex_attrib_array(1);
		gl.vertex_attrib_pointer_f32(1,3,glow::FLOAT,false,24,12);

        let u_mvp = gl.get_uniform_location(program, "u_mvp").unwrap();

        Self { program, vao, vbo, vertex_count: 0, u_mvp }
    }

    pub unsafe fn upload_vertices(&mut self, gl: &Context, verts: &[f32]) {
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
        gl.buffer_data_u8_slice(
            glow::ARRAY_BUFFER,
            bytemuck::cast_slice(verts),
            glow::STATIC_DRAW,
        );
        // 6 floats per vertex: xyz rgb
		self.vertex_count=(verts.len()/6)as i32;
    }

    pub unsafe fn paint(&self, gl: &Context, mvp: Matrix4<f32>) {
        gl.use_program(Some(self.program));
        gl.uniform_matrix_4_f32_slice(Some(&self.u_mvp), false, mvp.as_slice());
        gl.bind_vertex_array(Some(self.vao));
        gl.draw_arrays(glow::LINES, 0, self.vertex_count);
    }
}
