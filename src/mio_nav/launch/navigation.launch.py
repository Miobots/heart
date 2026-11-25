"""AMCL localisation on the saved map, plus Nav2 goals (H2.1).

Run after `sim.launch.py`:
    ros2 launch mio_nav navigation.launch.py
then click "Nav2 Goal" in RViz. This wraps nav2_bringup rather than re-describing its nodes;
everything robot-specific lives in config/nav2_params.yaml.

With `gateway:=true` it also starts mio_gateway, so the Brain can send the goals:
    DEV_TOKEN=... BRAIN_URL=ws://<brain>:8080/ws ros2 launch mio_nav navigation.launch.py gateway:=true
"""
import os
from ament_index_python.packages import get_package_share_directory
from launch import LaunchDescription
from launch.actions import DeclareLaunchArgument, IncludeLaunchDescription
from launch.conditions import IfCondition
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


def generate_launch_description():
    pkg_mio_nav = get_package_share_directory('mio_nav')
    pkg_nav2_bringup = get_package_share_directory('nav2_bringup')

    # The world the robot is in picks its saved map and named places; map:= / regions_file:= still
    # override either. Must match the world sim.launch.py was started with.
    world_arg = DeclareLaunchArgument(
        'world',
        default_value='home_arena',
        description='Which saved map and named places: home_arena or small_house'
    )
    world = LaunchConfiguration('world')
    map_arg = DeclareLaunchArgument(
        'map',
        default_value=PathJoinSubstitution([pkg_mio_nav, 'maps', [world, '.yaml']]),
        description='Full path to the saved map yaml'
    )
    params_arg = DeclareLaunchArgument(
        'params_file',
        default_value=os.path.join(pkg_mio_nav, 'config', 'nav2_params.yaml'),
        description='Full path to the Nav2 parameters file'
    )
    use_sim_time_arg = DeclareLaunchArgument(
        'use_sim_time',
        default_value='true',
        description='Use simulation (Gazebo) clock if true'
    )
    use_rviz_arg = DeclareLaunchArgument(
        'use_rviz',
        default_value='true',
        description='Launch RViz2 with the Nav2 panel if true'
    )

    gateway_arg = DeclareLaunchArgument(
        'gateway',
        default_value='false',
        description='Also start mio_gateway, which dials the Brain. Needs DEV_TOKEN in the '
                    'environment, and BRAIN_URL unless the Brain is at ws://127.0.0.1:8080/ws'
    )
    regions_arg = DeclareLaunchArgument(
        'regions_file',
        default_value=PathJoinSubstitution([pkg_mio_nav, 'maps', [world, '.regions.json']]),
        description='Where the named places of this map are, for goals the Brain sends by name'
    )

    use_sim_time = LaunchConfiguration('use_sim_time')

    nav2 = IncludeLaunchDescription(
        PythonLaunchDescriptionSource(os.path.join(pkg_nav2_bringup, 'launch', 'bringup_launch.py')),
        launch_arguments={
            'map': LaunchConfiguration('map'),
            'params_file': LaunchConfiguration('params_file'),
            'use_sim_time': use_sim_time,
            'autostart': 'true',
            # No keepout or speed masks exist for this map yet.
            'use_keepout_zones': 'False',
            'use_speed_zones': 'False',
        }.items()
    )

    rviz = Node(
        condition=IfCondition(LaunchConfiguration('use_rviz')),
        package='rviz2',
        executable='rviz2',
        name='rviz2',
        arguments=['-d', os.path.join(pkg_nav2_bringup, 'rviz', 'nav2_default_view.rviz')],
        parameters=[{'use_sim_time': use_sim_time}],
        output='screen',
    )

    # The only node that speaks to the Brain (H3.1). Off by default: with no Brain to dial it only
    # fills the log, and it will not start at all without a token.
    gateway = Node(
        condition=IfCondition(LaunchConfiguration('gateway')),
        package='mio_gateway',
        executable='mio_gateway',
        parameters=[{'regions_file': LaunchConfiguration('regions_file')}],
        output='screen',
    )

    return LaunchDescription([
        world_arg,
        map_arg,
        params_arg,
        use_sim_time_arg,
        use_rviz_arg,
        gateway_arg,
        regions_arg,
        nav2,
        rviz,
        gateway,
    ])
