import os
from ament_index_python.packages import get_package_share_directory
from launch import LaunchDescription
from launch.actions import AppendEnvironmentVariable, DeclareLaunchArgument, IncludeLaunchDescription, ExecuteProcess
from launch.conditions import IfCondition, UnlessCondition
from launch.launch_description_sources import PythonLaunchDescriptionSource
from launch.substitutions import LaunchConfiguration, PathJoinSubstitution
from launch_ros.actions import Node

# Sanitize Snap environment contamination if running from VS Code / terminal Snap
if 'LD_LIBRARY_PATH' in os.environ:
    os.environ['LD_LIBRARY_PATH'] = ':'.join([
        p for p in os.environ['LD_LIBRARY_PATH'].split(':')
        if '/snap/' not in p
    ])

for var in ['GTK_PATH', 'GTK_EXE_PREFIX', 'LOCPATH', 'GIO_MODULE_DIR', 'GSETTINGS_SCHEMA_DIR', 'GTK_IM_MODULE_FILE']:
    os.environ.pop(var, None)

if 'XDG_DATA_DIRS' in os.environ:
    os.environ['XDG_DATA_DIRS'] = ':'.join([
        p for p in os.environ['XDG_DATA_DIRS'].split(':')
        if '/snap/' not in p
    ])

# Render on the NVIDIA GPU when there is one. On a hybrid laptop Gazebo otherwise renders the camera
# and the GPU LiDAR on the integrated GPU, and in small_house the LiDAR then falls to ~8 Hz of its 10.
NVIDIA_EGL = '/usr/share/glvnd/egl_vendor.d/10_nvidia.json'
if os.path.exists(NVIDIA_EGL):
    os.environ.setdefault('__NV_PRIME_RENDER_OFFLOAD', '1')
    os.environ.setdefault('__GLX_VENDOR_LIBRARY_NAME', 'nvidia')
    os.environ.setdefault('__EGL_VENDOR_LIBRARY_FILENAMES', NVIDIA_EGL)

def generate_launch_description():
    pkg_mio_description = get_package_share_directory('mio_description')
    pkg_mio_sim = get_package_share_directory('mio_sim')

    default_bridge_config = os.path.join(pkg_mio_sim, 'config', 'ros_gz_bridge.yaml')

    # Launch Arguments
    world_arg = DeclareLaunchArgument(
        'world',
        default_value='home_arena',
        description='World in mio_sim/worlds, without .sdf: home_arena or small_house'
    )

    x_arg = DeclareLaunchArgument('x', default_value='0.0', description='Initial X position of robot')
    y_arg = DeclareLaunchArgument('y', default_value='0.0', description='Initial Y position of robot')
    z_arg = DeclareLaunchArgument('z', default_value='0.05', description='Initial Z position of robot')
    yaw_arg = DeclareLaunchArgument('yaw', default_value='0.0', description='Initial Yaw orientation of robot')

    headless_arg = DeclareLaunchArgument(
        'headless',
        default_value='false',
        description='Run Gazebo headless without GUI if true'
    )

    world = PathJoinSubstitution([pkg_mio_sim, 'worlds', [LaunchConfiguration('world'), '.sdf']])
    x = LaunchConfiguration('x')
    y = LaunchConfiguration('y')
    z = LaunchConfiguration('z')
    yaw = LaunchConfiguration('yaw')
    headless = LaunchConfiguration('headless')

    # 1. Include Robot State Publisher from mio_description
    rsp_launch = IncludeLaunchDescription(
        PythonLaunchDescriptionSource(
            os.path.join(pkg_mio_description, 'launch', 'rsp.launch.py')
        ),
        launch_arguments={'use_sim_time': 'true'}.items()
    )

    # 2. Start Gazebo Jetty (gz-sim 10) - GUI or Headless
    gz_sim_gui = ExecuteProcess(
        condition=UnlessCondition(headless),
        cmd=['gz', 'sim', '-r', world],
        output='screen'
    )

    gz_sim_headless = ExecuteProcess(
        condition=IfCondition(headless),
        cmd=['gz', 'sim', '-s', '-r', world],
        output='screen'
    )

    # 3. Spawn Robot in Gazebo from /robot_description topic
    spawn_robot = Node(
        package='ros_gz_sim',
        executable='create',
        output='screen',
        arguments=[
            '-topic', 'robot_description',
            '-name', 'mio',
            '-x', x,
            '-y', y,
            '-z', z,
            '-Y', yaw,
            '-allow_renaming', 'false'
        ]
    )

    # 4. ROS 2 <-> Gazebo Bridge
    ros_gz_bridge = Node(
        package='ros_gz_bridge',
        executable='parameter_bridge',
        output='screen',
        parameters=[{
            'config_file': default_bridge_config,
            'use_sim_time': True,
        }]
    )

    # 5. Safety supervisor (H2.3). The bridge forwards /cmd_vel_safe and nothing else, and this node
    #    is the only thing that publishes it: without it the robot does not move. If it dies it
    #    comes back having seen no command, which means it comes back publishing stop.
    safety_supervisor = Node(
        package='mio_safety_supervisor',
        executable='mio_safety_supervisor',
        output='screen',
        respawn=True,
        parameters=[{'use_sim_time': True}],
    )

    # model:// URIs (the small house's furniture, the obstacle box) resolve from mio_sim/models.
    model_path = AppendEnvironmentVariable(
        'GZ_SIM_RESOURCE_PATH', os.path.join(pkg_mio_sim, 'models'))

    return LaunchDescription([
        model_path,
        world_arg,
        x_arg,
        y_arg,
        z_arg,
        yaw_arg,
        headless_arg,
        rsp_launch,
        gz_sim_gui,
        gz_sim_headless,
        spawn_robot,
        ros_gz_bridge,
        safety_supervisor,
    ])
